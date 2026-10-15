//! Token-level expansion for each macro.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote, quote_spanned};
use syn::{
    Attribute, ImplItem, Item, ItemMod, LitStr, Meta, parse_quote, parse_quote_spanned,
    spanned::Spanned, visit_mut::VisitMut,
};

use solex_types::spec::{Decision, FeatureSpec, decide};

use crate::context::BuildContext;

fn err(e: syn::Error) -> TokenStream {
    e.to_compile_error()
}

pub fn channel(attr: TokenStream, item: TokenStream) -> TokenStream {
    let spec: FeatureSpec = match syn::parse2(attr) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let ctx = match BuildContext::load(Span::call_site()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    // Attribute macros may sit on free items or on associated items; try both.
    if let Ok(mut it) = syn::parse2::<Item>(item.clone()) {
        return match apply_to_item(&ctx, &spec, &mut it) {
            Ok(true) => match gate_nested_item(&ctx, &mut it) {
                Ok(()) => it.into_token_stream(),
                Err(e) => err(e),
            },
            Ok(false) => TokenStream::new(),
            Err(e) => err(e),
        };
    }
    match syn::parse2::<ImplItem>(item) {
        Ok(mut it) => match apply_to_impl_item(&ctx, &spec, &mut it) {
            Ok(true) => match gate_nested_impl_item(&ctx, &mut it) {
                Ok(()) => it.into_token_stream(),
                Err(e) => err(e),
            },
            Ok(false) => TokenStream::new(),
            Err(e) => err(e),
        },
        Err(e) => err(syn::Error::new(
            e.span(),
            "#[channel] can only be applied to items (functions, structs, modules, impls, ...)",
        )),
    }
}

/// Returns `Ok(true)` when the item stays, `Ok(false)` when it is removed.
fn apply_to_item(ctx: &BuildContext, spec: &FeatureSpec, item: &mut Item) -> syn::Result<bool> {
    match decide(spec, &ctx.tags) {
        Decision::Exclude => Ok(false),
        Decision::Include { epoch: None } => Ok(true),
        Decision::Include { epoch: Some(epoch) } => {
            match item {
                Item::Fn(f) => f.block.stmts.insert(0, epoch_gate(epoch, f.sig.span())),
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "an epoch bound (`[tag, epoch]`) is only supported on functions; use a \
                         plain tag for other items",
                    ));
                }
            }
            Ok(true)
        }
    }
}

fn apply_to_impl_item(
    ctx: &BuildContext,
    spec: &FeatureSpec,
    item: &mut ImplItem,
) -> syn::Result<bool> {
    match decide(spec, &ctx.tags) {
        Decision::Exclude => Ok(false),
        Decision::Include { epoch: None } => Ok(true),
        Decision::Include { epoch: Some(epoch) } => {
            match item {
                ImplItem::Fn(f) => f.block.stmts.insert(0, epoch_gate(epoch, f.sig.span())),
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "an epoch bound (`[tag, epoch]`) is only supported on functions",
                    ));
                }
            }
            Ok(true)
        }
    }
}

/// `solex::__private::require_epoch(N)?;` — `?` converts the `ProgramError`
/// into whatever error type the handler returns (Anchor's `Error` included).
fn epoch_gate(epoch: u64, span: Span) -> syn::Stmt {
    let lit = proc_macro2::Literal::u64_suffixed(epoch);
    parse_quote_spanned! { span => ::solex::__private::require_epoch(#lit)?; }
}

/// Is this attribute `#[channel(...)]` or `#[solex::channel(...)]`?
fn is_channel_attr(attr: &Attribute) -> bool {
    let path = attr.path();
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    matches!(segs.as_slice(), [f] | [_, f] if f == "channel")
}

fn take_channel_spec(attrs: &mut Vec<Attribute>) -> syn::Result<Option<FeatureSpec>> {
    let Some(pos) = attrs.iter().position(is_channel_attr) else {
        return Ok(None);
    };
    let attr = attrs.remove(pos);
    if attrs.iter().any(is_channel_attr) {
        return Err(syn::Error::new(
            attr.span(),
            "only one #[channel] attribute per item; list several tags in one attribute instead",
        ));
    }
    match &attr.meta {
        Meta::List(list) => Ok(Some(syn::parse2(list.tokens.clone())?)),
        _ => Err(syn::Error::new(
            attr.span(),
            "expected #[channel(...)] with at least one tag",
        )),
    }
}

fn item_attrs(item: &mut Item) -> Option<&mut Vec<Attribute>> {
    Some(match item {
        Item::Const(i) => &mut i.attrs,
        Item::Enum(i) => &mut i.attrs,
        Item::ExternCrate(i) => &mut i.attrs,
        Item::Fn(i) => &mut i.attrs,
        Item::ForeignMod(i) => &mut i.attrs,
        Item::Impl(i) => &mut i.attrs,
        Item::Macro(i) => &mut i.attrs,
        Item::Mod(i) => &mut i.attrs,
        Item::Static(i) => &mut i.attrs,
        Item::Struct(i) => &mut i.attrs,
        Item::Trait(i) => &mut i.attrs,
        Item::TraitAlias(i) => &mut i.attrs,
        Item::Type(i) => &mut i.attrs,
        Item::Union(i) => &mut i.attrs,
        Item::Use(i) => &mut i.attrs,
        _ => return None,
    })
}

pub fn program(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut module: ItemMod = match syn::parse2(item) {
        Ok(m) => m,
        Err(e) => return err(e),
    };
    let ctx = match BuildContext::load(Span::call_site()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    if module.content.is_none() {
        return err(syn::Error::new(
            module.span(),
            "#[solex::program] requires an inline module body (`mod name { ... }`)",
        ));
    }
    let mut gate = Gate::new(&ctx);
    gate.visit_item_mod_mut(&mut module);
    if let Some(e) = gate.error {
        return err(e);
    }

    let framework_attr = framework_program_attr(attr);
    quote! {
        #framework_attr
        #module
    }
}

/// The framework's own `#[program]`, re-exported by the `solex` crate according
/// to its cargo features (Anchor's macro, or a passthrough for native programs).
fn framework_program_attr(attr: TokenStream) -> TokenStream {
    if attr.is_empty() {
        quote!(#[::solex::__private::framework_program])
    } else {
        quote!(#[::solex::__private::framework_program(#attr)])
    }
}

pub fn declare_id(input: TokenStream) -> TokenStream {
    if !input.is_empty() {
        return err(syn::Error::new_spanned(
            input,
            "solex::declare_id! takes no arguments; the id comes from Solex.yaml for the active \
             channel",
        ));
    }
    let span = Span::call_site();
    let ctx = match BuildContext::load(span) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    let id = match ctx.program_id(span) {
        Ok(id) => id,
        Err(e) => return err(e),
    };
    let lit = LitStr::new(&id, span);
    declare_id_tokens(lit)
}

fn declare_id_tokens(lit: LitStr) -> TokenStream {
    quote_spanned! { lit.span() => ::solex::__private::framework_declare_id!(#lit); }
}

/// Native `declare_id!`: `ID`, `id()` and `check_id()` like Anchor's.
pub fn native_declare_id(input: TokenStream) -> TokenStream {
    let lit: LitStr = match syn::parse2(input) {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    quote_spanned! { lit.span() =>
        /// Program id of the active Solex channel.
        pub const ID: ::solex::Pubkey = ::solex::Pubkey::from_str_const(#lit);

        /// Program id of the active Solex channel.
        pub fn id() -> ::solex::Pubkey {
            ID
        }

        /// Whether `id` is this program's id.
        pub fn check_id(id: &::solex::Pubkey) -> bool {
            id == &ID
        }
    }
}

pub fn active_channel(input: TokenStream) -> TokenStream {
    if !input.is_empty() {
        return err(syn::Error::new_spanned(
            input,
            "solex::active_channel! takes no arguments",
        ));
    }
    let ctx = match BuildContext::load(Span::call_site()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    let lit = LitStr::new(&ctx.channel, Span::call_site());
    quote!(#lit)
}

/// Active cluster name, or `""` when the channel does not pin one.
pub fn active_cluster(input: TokenStream) -> TokenStream {
    if !input.is_empty() {
        return err(syn::Error::new_spanned(
            input,
            "solex::active_cluster! takes no arguments",
        ));
    }
    let ctx = match BuildContext::load(Span::call_site()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    let lit = LitStr::new(ctx.cluster.as_deref().unwrap_or(""), Span::call_site());
    quote!(#lit)
}

pub fn ships(input: TokenStream) -> TokenStream {
    let tag = match syn::parse2::<syn::Ident>(input.clone()) {
        Ok(ident) => ident.to_string(),
        Err(_) => match syn::parse2::<LitStr>(input) {
            Ok(lit) => lit.value(),
            Err(e) => {
                return err(syn::Error::new(
                    e.span(),
                    "solex::ships! expects one tag, e.g. ships!(canary)",
                ));
            }
        },
    };
    let ctx = match BuildContext::load(Span::call_site()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    let value = ctx.tags.contains(&tag);
    quote!(#value)
}

/// `#[solex::activation("hooks")]`: when the manifest gives `hooks` an
/// activation epoch on the active channel, inject the runtime gate; otherwise
/// leave the function untouched.
pub fn activation(attr: TokenStream, item: TokenStream) -> TokenStream {
    let feature: LitStr = match syn::parse2(attr) {
        Ok(l) => l,
        Err(e) => {
            return err(syn::Error::new(
                e.span(),
                "#[solex::activation] expects the feature name, e.g. #[solex::activation(\"hooks\")]",
            ));
        }
    };
    let ctx = match BuildContext::load(feature.span()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    let epoch = match ctx.activation_epoch(&feature.value(), feature.span()) {
        Ok(e) => e,
        Err(e) => return err(e),
    };
    let Some(epoch) = epoch else {
        return item;
    };
    let mut f: syn::ItemFn = match syn::parse2(item.clone()) {
        Ok(f) => f,
        Err(_) => match syn::parse2::<syn::ImplItemFn>(item) {
            Ok(mut m) => {
                m.block.stmts.insert(0, epoch_gate(epoch, m.sig.span()));
                return m.into_token_stream();
            }
            Err(e) => {
                return err(syn::Error::new(
                    e.span(),
                    "#[solex::activation] applies to functions",
                ));
            }
        },
    };
    f.block.stmts.insert(0, epoch_gate(epoch, f.sig.span()));
    f.into_token_stream()
}

pub fn gated(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return err(syn::Error::new_spanned(
            attr,
            "#[solex::gated] takes no arguments; put #[channel(...)] on the variants, fields, \
             arms or statements inside",
        ));
    }
    let ctx = match BuildContext::load(Span::call_site()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    if let Ok(mut it) = syn::parse2::<Item>(item.clone()) {
        return match gate_nested_item(&ctx, &mut it) {
            Ok(()) => it.into_token_stream(),
            Err(e) => err(e),
        };
    }
    match syn::parse2::<ImplItem>(item) {
        Ok(mut it) => match gate_nested_impl_item(&ctx, &mut it) {
            Ok(()) => it.into_token_stream(),
            Err(e) => err(e),
        },
        Err(e) => err(syn::Error::new(
            e.span(),
            "#[solex::gated] can only be applied to items",
        )),
    }
}

pub fn var(input: TokenStream) -> TokenStream {
    let name: LitStr = match syn::parse2(input) {
        Ok(l) => l,
        Err(e) => {
            return err(syn::Error::new(
                e.span(),
                "solex::var! expects a string literal, e.g. var!(\"treasury_owner\")",
            ));
        }
    };
    let ctx = match BuildContext::load(name.span()) {
        Ok(c) => c,
        Err(e) => return err(e),
    };
    match ctx.var(&name.value(), name.span()) {
        Ok(v) => {
            let lit = LitStr::new(&v, name.span());
            quote!(#lit)
        }
        Err(e) => err(e),
    }
}

fn gate_nested_item(ctx: &BuildContext, item: &mut Item) -> syn::Result<()> {
    let mut gate = Gate::new(ctx);
    gate.visit_item_mut(item);
    gate.error.map_or(Ok(()), Err)
}

fn gate_nested_impl_item(ctx: &BuildContext, item: &mut ImplItem) -> syn::Result<()> {
    let mut gate = Gate::new(ctx);
    gate.visit_impl_item_mut(item);
    gate.error.map_or(Ok(()), Err)
}

/// Walks an item and resolves every nested `#[channel(...)]` marker.
///
/// Items (in modules, impl blocks and fn bodies) are removed or kept with the
/// usual epoch handling. Positions where attribute macros cannot run (enum
/// variants, struct fields, match arms, `let` statements, macro statements)
/// get `#[cfg(any())]` when excluded and lose the marker when included.
struct Gate<'a> {
    ctx: &'a BuildContext,
    error: Option<syn::Error>,
}

impl<'a> Gate<'a> {
    fn new(ctx: &'a BuildContext) -> Self {
        Self { ctx, error: None }
    }

    fn fail(&mut self, e: syn::Error) {
        if self.error.is_none() {
            self.error = Some(e);
        }
    }

    /// Rewrite a marker on a non-item position into a `cfg`.
    fn cfg_attrs(&mut self, attrs: &mut Vec<Attribute>) {
        let spec = match take_channel_spec(attrs) {
            Ok(Some(spec)) => spec,
            Ok(None) => return,
            Err(e) => return self.fail(e),
        };
        match decide(&spec, &self.ctx.tags) {
            Decision::Exclude => attrs.push(parse_quote!(#[cfg(any())])),
            Decision::Include { epoch: None } => {}
            Decision::Include { epoch: Some(_) } => self.fail(syn::Error::new(
                spec.entries[0].span,
                "an epoch bound (`[tag, epoch]`) is only supported on functions; variants, \
                 fields, arms and statements take a plain tag",
            )),
        }
    }

    /// Returns whether the item stays; applies epoch gates in place.
    fn keep_item(&mut self, item: &mut Item) -> bool {
        let spec = match item_attrs(item).map(take_channel_spec) {
            Some(Ok(Some(spec))) => spec,
            Some(Ok(None)) | None => return true,
            Some(Err(e)) => {
                self.fail(e);
                return true;
            }
        };
        match apply_to_item(self.ctx, &spec, item) {
            Ok(keep) => keep,
            Err(e) => {
                self.fail(e);
                true
            }
        }
    }

    fn keep_impl_item(&mut self, item: &mut ImplItem) -> bool {
        let attrs = match item {
            ImplItem::Const(i) => &mut i.attrs,
            ImplItem::Fn(i) => &mut i.attrs,
            ImplItem::Type(i) => &mut i.attrs,
            ImplItem::Macro(i) => &mut i.attrs,
            _ => return true,
        };
        let spec = match take_channel_spec(attrs) {
            Ok(Some(spec)) => spec,
            Ok(None) => return true,
            Err(e) => {
                self.fail(e);
                return true;
            }
        };
        match apply_to_impl_item(self.ctx, &spec, item) {
            Ok(keep) => keep,
            Err(e) => {
                self.fail(e);
                true
            }
        }
    }
}

impl VisitMut for Gate<'_> {
    fn visit_item_mod_mut(&mut self, m: &mut ItemMod) {
        if let Some((_, items)) = m.content.as_mut() {
            items.retain_mut(|it| self.keep_item(it));
        }
        syn::visit_mut::visit_item_mod_mut(self, m);
    }

    fn visit_item_impl_mut(&mut self, i: &mut syn::ItemImpl) {
        i.items.retain_mut(|it| self.keep_impl_item(it));
        syn::visit_mut::visit_item_impl_mut(self, i);
    }

    fn visit_item_enum_mut(&mut self, e: &mut syn::ItemEnum) {
        for v in e.variants.iter_mut() {
            self.cfg_attrs(&mut v.attrs);
        }
        syn::visit_mut::visit_item_enum_mut(self, e);
    }

    fn visit_field_mut(&mut self, f: &mut syn::Field) {
        self.cfg_attrs(&mut f.attrs);
        syn::visit_mut::visit_field_mut(self, f);
    }

    fn visit_expr_match_mut(&mut self, m: &mut syn::ExprMatch) {
        for arm in m.arms.iter_mut() {
            self.cfg_attrs(&mut arm.attrs);
        }
        syn::visit_mut::visit_expr_match_mut(self, m);
    }

    fn visit_block_mut(&mut self, b: &mut syn::Block) {
        b.stmts.retain_mut(|stmt| match stmt {
            syn::Stmt::Item(item) => self.keep_item(item),
            syn::Stmt::Local(local) => {
                self.cfg_attrs(&mut local.attrs);
                true
            }
            syn::Stmt::Macro(mac) => {
                self.cfg_attrs(&mut mac.attrs);
                true
            }
            syn::Stmt::Expr(expr, _) => {
                if let Some(attrs) = expr_attrs(expr) {
                    self.cfg_attrs(attrs);
                }
                true
            }
        });
        syn::visit_mut::visit_block_mut(self, b);
    }
}

/// Outer attributes of an expression statement, for the expression kinds that
/// make sense to gate. Other kinds keep their marker and rustc reports it.
fn expr_attrs(expr: &mut syn::Expr) -> Option<&mut Vec<Attribute>> {
    use syn::Expr::*;
    Some(match expr {
        Assign(e) => &mut e.attrs,
        Block(e) => &mut e.attrs,
        Call(e) => &mut e.attrs,
        ForLoop(e) => &mut e.attrs,
        If(e) => &mut e.attrs,
        Loop(e) => &mut e.attrs,
        Macro(e) => &mut e.attrs,
        Match(e) => &mut e.attrs,
        MethodCall(e) => &mut e.attrs,
        Return(e) => &mut e.attrs,
        Try(e) => &mut e.attrs,
        Unsafe(e) => &mut e.attrs,
        While(e) => &mut e.attrs,
        _ => return None,
    })
}
