//! Static scan of a program's sources for everything solex gates: `#[channel(...)]`
//! attributes and `#[cfg(feature = "...")]` predicates on channel-managed
//! features. The result is the per-channel matrix behind `solex features`.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use quote::ToTokens;
use solex_types::{
    Channels,
    spec::{Decision, FeatureSpec, decide},
};
use syn::{Attribute, Meta, spanned::Spanned, visit::Visit};
use walkdir::WalkDir;

use crate::{CoreError, Result};

/// A `cfg` predicate, reduced to what matters for channels: feature atoms.
/// Anything else (`target_os`, `test`, ...) is treated as true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfgExpr {
    Feature(String),
    Any(Vec<CfgExpr>),
    All(Vec<CfgExpr>),
    Not(Box<CfgExpr>),
    Other,
}

impl CfgExpr {
    pub fn parse(meta: &Meta) -> Option<CfgExpr> {
        match meta {
            Meta::NameValue(nv) if nv.path.is_ident("feature") => match &nv.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) => Some(CfgExpr::Feature(s.value())),
                _ => None,
            },
            Meta::List(list) => {
                let parser = syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated;
                let inner = list.parse_args_with(parser).ok()?;
                let items: Vec<CfgExpr> = inner.iter().filter_map(CfgExpr::parse).collect();
                if list.path.is_ident("any") {
                    Some(CfgExpr::Any(items))
                } else if list.path.is_ident("all") {
                    Some(CfgExpr::All(items))
                } else if list.path.is_ident("not") {
                    items.into_iter().next().map(|e| CfgExpr::Not(Box::new(e)))
                } else {
                    Some(CfgExpr::Other)
                }
            }
            _ => Some(CfgExpr::Other),
        }
    }

    /// Feature names mentioned anywhere in the predicate.
    pub fn features(&self, out: &mut BTreeSet<String>) {
        match self {
            CfgExpr::Feature(f) => {
                out.insert(f.clone());
            }
            CfgExpr::Any(v) | CfgExpr::All(v) => v.iter().for_each(|e| e.features(out)),
            CfgExpr::Not(e) => e.features(out),
            CfgExpr::Other => {}
        }
    }

    pub fn eval(&self, enabled: &[String]) -> bool {
        match self {
            CfgExpr::Feature(f) => enabled.iter().any(|e| e == f),
            CfgExpr::Any(v) => v.iter().any(|e| e.eval(enabled)),
            CfgExpr::All(v) => v.iter().all(|e| e.eval(enabled)),
            CfgExpr::Not(e) => !e.eval(enabled),
            CfgExpr::Other => true,
        }
    }

    pub fn text(&self) -> String {
        match self {
            CfgExpr::Feature(f) => format!("feature = \"{f}\""),
            CfgExpr::Any(v) => format!(
                "any({})",
                v.iter().map(|e| e.text()).collect::<Vec<_>>().join(", ")
            ),
            CfgExpr::All(v) => format!(
                "all({})",
                v.iter().map(|e| e.text()).collect::<Vec<_>>().join(", ")
            ),
            CfgExpr::Not(e) => format!("not({})", e.text()),
            CfgExpr::Other => "..".into(),
        }
    }
}

/// What gates an item.
#[derive(Debug, Clone)]
pub enum Gate {
    /// `#[channel(...)]`: channel tags, optionally epoch-bound.
    Channel(FeatureSpec),
    /// `#[cfg(...)]` mentioning at least one channel-managed feature.
    Cfg(CfgExpr),
}

/// One gated item found in the sources.
#[derive(Debug, Clone)]
pub struct GatedItem {
    pub file: PathBuf,
    pub line: usize,
    pub kind: &'static str,
    pub name: String,
    /// The gate as written: `devnet`, `[nightly, 850]` or `feature = "hooks"`.
    pub gate_text: String,
    /// The feature this item belongs to, when the gate is exactly one feature.
    pub feature: Option<String>,
    pub gate: Gate,
}

impl GatedItem {
    pub fn decision(&self, channels: &Channels, channel: &str) -> Result<Decision> {
        Ok(match &self.gate {
            Gate::Channel(spec) => decide(spec, &channels.tags(channel)?),
            Gate::Cfg(expr) => {
                if expr.eval(&channels.features_for(channel)?) {
                    Decision::Include { epoch: None }
                } else {
                    Decision::Exclude
                }
            }
        })
    }
}

/// Result of a scan: gated items plus bookkeeping for `--check`.
#[derive(Debug, Default)]
pub struct Scan {
    pub items: Vec<GatedItem>,
    /// Cargo features seen in `cfg` that the manifest does not manage
    /// (`idl`, `no-entrypoint`, ...). Informational.
    pub unmanaged_features: BTreeSet<String>,
}

/// Walk `src/` under `program_dir` and collect gated items in deterministic
/// (path, line) order.
pub fn scan_program(program_dir: &Path, channels: &Channels) -> Result<Scan> {
    let src = program_dir.join("src");
    let root = if src.is_dir() {
        src
    } else {
        program_dir.to_path_buf()
    };
    let mut files: Vec<PathBuf> = WalkDir::new(&root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();

    let mut scan = Scan::default();
    for file in files {
        let part = scan_file(&file, program_dir, channels)?;
        scan.items.extend(part.items);
        scan.unmanaged_features.extend(part.unmanaged_features);
    }
    Ok(scan)
}

pub fn scan_file(path: &Path, display_base: &Path, channels: &Channels) -> Result<Scan> {
    let source = std::fs::read_to_string(path).map_err(|e| CoreError::io("read", path, e))?;
    let ast = syn::parse_file(&source).map_err(|source| CoreError::Syntax {
        path: path.to_path_buf(),
        source,
    })?;
    let display = path
        .strip_prefix(display_base)
        .unwrap_or(path)
        .to_path_buf();
    let managed: BTreeSet<String> = channels.features.keys().cloned().collect();
    let mut visitor = Collector {
        file: display,
        managed,
        scan: Scan::default(),
        error: None,
    };
    visitor.visit_file(&ast);
    if let Some(e) = visitor.error {
        return Err(CoreError::Syntax {
            path: path.to_path_buf(),
            source: e,
        });
    }
    Ok(visitor.scan)
}

struct Collector {
    file: PathBuf,
    managed: BTreeSet<String>,
    scan: Scan,
    error: Option<syn::Error>,
}

fn is_channel_attr(attr: &Attribute) -> bool {
    let segs: Vec<String> = attr
        .path()
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    matches!(segs.as_slice(), [f] | [_, f] if f == "channel")
}

impl Collector {
    fn record(
        &mut self,
        attrs: &[Attribute],
        kind: &'static str,
        name: String,
        span: proc_macro2::Span,
    ) {
        for attr in attrs {
            let Meta::List(list) = &attr.meta else {
                continue;
            };
            if is_channel_attr(attr) {
                match syn::parse2::<FeatureSpec>(list.tokens.clone()) {
                    Ok(spec) => self.scan.items.push(GatedItem {
                        file: self.file.clone(),
                        line: span.start().line,
                        kind,
                        name: name.clone(),
                        gate_text: spec.tags_text(),
                        feature: None,
                        gate: Gate::Channel(spec),
                    }),
                    Err(e) => {
                        if self.error.is_none() {
                            self.error = Some(e);
                        }
                    }
                }
            } else if attr.path().is_ident("cfg") {
                let Ok(meta) = list.parse_args::<Meta>() else {
                    continue;
                };
                let Some(expr) = CfgExpr::parse(&meta) else {
                    continue;
                };
                let mut mentioned = BTreeSet::new();
                expr.features(&mut mentioned);
                if mentioned.is_empty() {
                    continue;
                }
                let (managed, unmanaged): (Vec<_>, Vec<_>) = mentioned
                    .into_iter()
                    .partition(|f| self.managed.contains(f));
                self.scan.unmanaged_features.extend(unmanaged);
                if managed.is_empty() {
                    continue;
                }
                let feature = match &expr {
                    CfgExpr::Feature(f) => Some(f.clone()),
                    _ => None,
                };
                self.scan.items.push(GatedItem {
                    file: self.file.clone(),
                    line: span.start().line,
                    kind,
                    name: name.clone(),
                    gate_text: expr.text(),
                    feature,
                    gate: Gate::Cfg(expr),
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        self.record(&i.attrs, "fn", i.sig.ident.to_string(), i.sig.span());
        syn::visit::visit_item_fn(self, i);
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        self.record(&i.attrs, "method", i.sig.ident.to_string(), i.sig.span());
        syn::visit::visit_impl_item_fn(self, i);
    }
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        self.record(&i.attrs, "struct", i.ident.to_string(), i.ident.span());
        syn::visit::visit_item_struct(self, i);
    }
    fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
        self.record(&i.attrs, "enum", i.ident.to_string(), i.ident.span());
        syn::visit::visit_item_enum(self, i);
    }
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        self.record(&i.attrs, "mod", i.ident.to_string(), i.ident.span());
        syn::visit::visit_item_mod(self, i);
    }
    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        self.record(&i.attrs, "const", i.ident.to_string(), i.ident.span());
        syn::visit::visit_item_const(self, i);
    }
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let name = type_name(&i.self_ty);
        self.record(&i.attrs, "impl", name, i.impl_token.span());
        syn::visit::visit_item_impl(self, i);
    }
    fn visit_variant(&mut self, v: &'ast syn::Variant) {
        self.record(&v.attrs, "variant", v.ident.to_string(), v.ident.span());
        syn::visit::visit_variant(self, v);
    }
    fn visit_arm(&mut self, a: &'ast syn::Arm) {
        let pat = a.pat.to_token_stream().to_string().replace(" :: ", "::");
        self.record(&a.attrs, "arm", pat, a.pat.span());
        syn::visit::visit_arm(self, a);
    }
    fn visit_field(&mut self, f: &'ast syn::Field) {
        let name = f
            .ident
            .as_ref()
            .map(|i| i.to_string())
            .unwrap_or_else(|| "_".into());
        self.record(&f.attrs, "field", name, f.span());
        syn::visit::visit_field(self, f);
    }
}

fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "?".into()),
        _ => "?".into(),
    }
}

/// Resolved cell of the feature matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Excluded,
    Included,
    IncludedFrom(u64),
}

impl From<Decision> for Cell {
    fn from(d: Decision) -> Self {
        match d {
            Decision::Exclude => Cell::Excluded,
            Decision::Include { epoch: None } => Cell::Included,
            Decision::Include { epoch: Some(e) } => Cell::IncludedFrom(e),
        }
    }
}

/// `items × channels` matrix.
pub struct Matrix {
    pub channels: Vec<String>,
    pub rows: Vec<(GatedItem, Vec<Cell>)>,
}

pub fn matrix(config: &Channels, items: Vec<GatedItem>, channels: &[String]) -> Result<Matrix> {
    let mut cells_for: BTreeMap<String, Vec<Cell>> = BTreeMap::new();
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        let mut cells = Vec::with_capacity(channels.len());
        for ch in channels {
            let mut cell = Cell::from(item.decision(config, ch)?);
            // Cargo features carry activation epochs from the manifest, not
            // from the attribute: surface them in the matrix too.
            if let (Gate::Cfg(_), Some(feature), Cell::Included) = (&item.gate, &item.feature, cell)
            {
                if let Some(epoch) = config.activation_epoch(feature, ch)? {
                    cell = Cell::IncludedFrom(epoch);
                }
            }
            cells.push(cell);
        }
        cells_for
            .entry(item.gate_text.clone())
            .or_insert_with(|| cells.clone());
        rows.push((item, cells));
    }
    Ok(Matrix {
        channels: channels.to_vec(),
        rows,
    })
}

/// `#[channel]` tags that are not channels: almost always a typo.
pub fn unknown_tags(config: &Channels, items: &[GatedItem]) -> Vec<(String, GatedItem)> {
    let known = config.known_tags();
    let mut out = Vec::new();
    for item in items {
        if let Gate::Channel(spec) = &item.gate {
            for entry in &spec.entries {
                if !known.contains(&entry.tag) {
                    out.push((entry.tag.clone(), item.clone()));
                }
            }
        }
    }
    out
}

/// Features declared in the manifest that no `cfg` in the sources mentions.
pub fn unused_features(config: &Channels, items: &[GatedItem]) -> Vec<String> {
    let mut used = BTreeSet::new();
    for item in items {
        if let Gate::Cfg(expr) = &item.gate {
            expr.features(&mut used);
        }
    }
    config
        .features
        .keys()
        .filter(|f| !used.contains(*f))
        .cloned()
        .collect()
}

/// Features declared in the manifest but missing from the program's
/// `Cargo.toml` `[features]`; a build with `--features` would fail.
pub fn undeclared_in_cargo(config: &Channels, cargo_toml: &Path) -> Result<Vec<String>> {
    let text =
        std::fs::read_to_string(cargo_toml).map_err(|e| CoreError::io("read", cargo_toml, e))?;
    let doc: toml::Table = toml::from_str(&text)
        .map_err(|e| CoreError::message(format!("invalid {}: {e}", cargo_toml.display())))?;
    let declared: BTreeSet<String> = doc
        .get("features")
        .and_then(|f| f.as_table())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    Ok(config
        .features
        .keys()
        .filter(|f| !declared.contains(*f))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
        solex::declare_id!();

        #[solex::channel(nightly)]
        pub struct Thing;

        impl Thing {
            #[solex::channel([stable, 10])]
            fn m(&self) {}
        }

        #[cfg(feature = "hooks")]
        pub struct Hook;

        #[cfg(all(feature = "hooks", not(feature = "idl")))]
        pub struct HookIdl;

        #[cfg(feature = "idl")]
        pub struct IdlOnly;

        #[solex::program]
        mod program {
            #[channel(stable)]
            pub fn a() {}
            #[channel([nightly, 850], canary)]
            pub fn b() {}
            pub fn plain() {}
        }

        enum Ix {
            Open = 1,
            #[cfg(feature = "hooks")]
            OpenWithHooks = 10,
        }
    "#;

    fn manifest() -> Channels {
        let toml = r#"
[channels.stable]
program_id = "1"
[channels.nightly]
includes = ["stable"]
program_id = "1"
[channels.canary]
includes = ["nightly"]
program_id = "1"

[features.hooks]
channels = ["nightly"]
epoch = { canary = 42 }
"#;
        solex_types::parse(toml, Path::new("Solex.toml"), solex_types::Format::Toml)
            .unwrap()
            .0
            .channels
    }

    fn scanned() -> (Scan, Channels) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, SRC).unwrap();
        let m = manifest();
        (scan_file(&file, dir.path(), &m).unwrap(), m)
    }

    #[test]
    fn finds_channel_attrs_and_managed_cfgs() {
        let (scan, _) = scanned();
        let got: Vec<_> = scan
            .items
            .iter()
            .map(|i| (i.kind, i.name.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("struct", "Thing"),
                ("method", "m"),
                ("struct", "Hook"),
                ("struct", "HookIdl"),
                ("fn", "a"),
                ("fn", "b"),
                ("variant", "OpenWithHooks"),
            ]
        );
        assert_eq!(scan.items[2].feature.as_deref(), Some("hooks"));
        assert_eq!(scan.items[3].feature, None);
        assert_eq!(
            scan.items[3].gate_text,
            "all(feature = \"hooks\", not(feature = \"idl\"))"
        );
        assert_eq!(
            scan.unmanaged_features.into_iter().collect::<Vec<_>>(),
            ["idl"]
        );
    }

    #[test]
    fn matrix_resolves_features_and_epochs_per_channel() {
        let (scan, m) = scanned();
        let channels = m.names();
        let mx = matrix(&m, scan.items, &channels).unwrap();
        let hook = mx.rows.iter().find(|(i, _)| i.name == "Hook").unwrap();
        assert_eq!(
            hook.1,
            [Cell::Excluded, Cell::Included, Cell::IncludedFrom(42)]
        );
        let b = mx.rows.iter().find(|(i, _)| i.name == "b").unwrap();
        assert_eq!(
            b.1,
            [Cell::Excluded, Cell::IncludedFrom(850), Cell::Included]
        );
    }

    #[test]
    fn reports_unknown_tags_and_unused_features() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "#[solex::channel(nightlyy)] fn x() {}").unwrap();
        let m = manifest();
        let scan = scan_file(&file, dir.path(), &m).unwrap();
        let unknown = unknown_tags(&m, &scan.items);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].0, "nightlyy");
        assert_eq!(unused_features(&m, &scan.items), ["hooks"]);
    }

    #[test]
    fn checks_cargo_declares_managed_features() {
        let dir = tempfile::tempdir().unwrap();
        let cargo = dir.path().join("Cargo.toml");
        std::fs::write(&cargo, "[package]\nname = \"x\"\n[features]\nidl = []\n").unwrap();
        assert_eq!(undeclared_in_cargo(&manifest(), &cargo).unwrap(), ["hooks"]);
        std::fs::write(&cargo, "[package]\nname = \"x\"\n[features]\nhooks = []\n").unwrap();
        assert!(undeclared_in_cargo(&manifest(), &cargo).unwrap().is_empty());
    }
}
