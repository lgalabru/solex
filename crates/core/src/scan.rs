//! Static scan of a program's sources for `#[channel(...)]` attributes, and the
//! per-channel matrix of what each one resolves to. This is what lets
//! `solex features` answer "what ships where" without compiling anything.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use solex_types::{
    Channels,
    spec::{Decision, FeatureSpec, decide},
};
use syn::{Attribute, Meta, spanned::Spanned, visit::Visit};
use walkdir::WalkDir;

use crate::{CoreError, Result};

/// One gated item found in the sources.
#[derive(Debug, Clone)]
pub struct GatedItem {
    pub file: PathBuf,
    pub line: usize,
    pub kind: &'static str,
    pub name: String,
    /// Human-readable spelling of the attribute arguments.
    pub spec_text: String,
    pub spec: FeatureSpec,
}

impl GatedItem {
    pub fn decision(&self, channels: &Channels, channel: &str) -> Result<Decision> {
        let tags = channels.tags(channel)?;
        Ok(decide(&self.spec, &tags))
    }
}

/// Walk `src/` under `program_dir` and collect gated items in deterministic
/// (path, line) order.
pub fn scan_program(program_dir: &Path) -> Result<Vec<GatedItem>> {
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

    let mut items = Vec::new();
    for file in files {
        items.extend(scan_file(&file, program_dir)?);
    }
    Ok(items)
}

pub fn scan_file(path: &Path, display_base: &Path) -> Result<Vec<GatedItem>> {
    let source = std::fs::read_to_string(path).map_err(|e| CoreError::io("read", path, e))?;
    let ast = syn::parse_file(&source).map_err(|source| CoreError::Syntax {
        path: path.to_path_buf(),
        source,
    })?;
    let display = path
        .strip_prefix(display_base)
        .unwrap_or(path)
        .to_path_buf();
    let mut visitor = Collector {
        file: display,
        items: Vec::new(),
        error: None,
    };
    visitor.visit_file(&ast);
    if let Some(e) = visitor.error {
        return Err(CoreError::Syntax {
            path: path.to_path_buf(),
            source: e,
        });
    }
    Ok(visitor.items)
}

struct Collector {
    file: PathBuf,
    items: Vec<GatedItem>,
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
        for attr in attrs.iter().filter(|a| is_channel_attr(a)) {
            let Meta::List(list) = &attr.meta else {
                continue;
            };
            match syn::parse2::<FeatureSpec>(list.tokens.clone()) {
                Ok(spec) => self.items.push(GatedItem {
                    file: self.file.clone(),
                    line: span.start().line,
                    kind,
                    name: name.clone(),
                    spec_text: list.tokens.to_string().replace(" ,", ","),
                    spec,
                }),
                Err(e) => {
                    if self.error.is_none() {
                        self.error = Some(e);
                    }
                }
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
    let mut tag_sets = BTreeMap::new();
    for ch in channels {
        tag_sets.insert(ch.clone(), config.tags(ch)?);
    }
    let rows = items
        .into_iter()
        .map(|item| {
            let cells = channels
                .iter()
                .map(|ch| Cell::from(decide(&item.spec, &tag_sets[ch])))
                .collect();
            (item, cells)
        })
        .collect();
    Ok(Matrix {
        channels: channels.to_vec(),
        rows,
    })
}

/// Tags referenced by attributes that no channel defines: almost always a typo,
/// and such an item would never ship anywhere.
pub fn unknown_tags(config: &Channels, items: &[GatedItem]) -> Vec<(String, GatedItem)> {
    let mut out = Vec::new();
    for item in items {
        for entry in &item.spec.entries {
            if !config.channels.contains_key(&entry.tag) {
                out.push((entry.tag.clone(), item.clone()));
            }
        }
    }
    out
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

        #[solex::program]
        mod program {
            #[channel(stable)]
            pub fn a() {}
            #[channel([nightly, 850], canary)]
            pub fn b() {}
            pub fn plain() {}
        }
    "#;

    fn manifest() -> Channels {
        let yaml = r#"
name: t
channels:
  stable: { program_id: "1" }
  nightly: { includes: [stable], program_id: "1" }
  canary: { includes: [nightly], program_id: "1" }
"#;
        solex_types::parse(yaml, Path::new("Solex.yaml"), solex_types::Format::Yaml)
            .unwrap()
            .0
            .channels
    }

    #[test]
    fn finds_items_in_order_with_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, SRC).unwrap();
        let items = scan_file(&file, dir.path()).unwrap();
        let got: Vec<_> = items.iter().map(|i| (i.kind, i.name.as_str())).collect();
        assert_eq!(
            got,
            [
                ("struct", "Thing"),
                ("method", "m"),
                ("fn", "a"),
                ("fn", "b")
            ]
        );
        assert_eq!(items[0].file, Path::new("lib.rs"));
        assert!(items[0].line > 0);
    }

    #[test]
    fn matrix_resolves_per_channel() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, SRC).unwrap();
        let items = scan_file(&file, dir.path()).unwrap();
        let m = manifest();
        let channels = m.names();
        let mx = matrix(&m, items, &channels).unwrap();
        // channels keep declaration order: stable, nightly, canary
        let b = mx.rows.iter().find(|(i, _)| i.name == "b").unwrap();
        assert_eq!(
            b.1,
            [Cell::Excluded, Cell::IncludedFrom(850), Cell::Included]
        );
        let thing = mx.rows.iter().find(|(i, _)| i.name == "Thing").unwrap();
        assert_eq!(thing.1, [Cell::Excluded, Cell::Included, Cell::Included]);
    }

    #[test]
    fn reports_unknown_tags() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "#[solex::channel(nightlyy)] fn x() {}").unwrap();
        let items = scan_file(&file, dir.path()).unwrap();
        let unknown = unknown_tags(&manifest(), &items);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].0, "nightlyy");
    }
}
