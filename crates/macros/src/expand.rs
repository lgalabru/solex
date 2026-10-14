//! Token-level expansion for each macro.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote, quote_spanned};
use syn::{
    Attribute, ImplItem, Item, ItemMod, LitStr, Meta, parse_quote_spanned, spanned::Spanned,
};

use solex_manifest::spec::{Decision, FeatureSpec, decide};

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
            Ok(true) => it.into_token_stream(),
            Ok(false) => TokenStream::new(),
            Err(e) => err(e),
        };
    }
    match syn::parse2::<ImplItem>(item) {
        Ok(mut it) => match apply_to_impl_item(&ctx, &spec, &mut it) {
            Ok(true) => it.into_token_stream(),
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
    let Some((_, items)) = module.content.as_mut() else {
        return err(syn::Error::new(
            module.span(),
            "#[solex::program] requires an inline module body (`mod name { ... }`)",
        ));
    };

    let mut kept = Vec::with_capacity(items.len());
    for mut it in items.drain(..) {
        let spec = match item_attrs(&mut it).map(take_channel_spec) {
            Some(Ok(spec)) => spec,
            Some(Err(e)) => return err(e),
            None => None,
        };
        let keep = match spec {
            None => true,
            Some(spec) => match apply_to_item(&ctx, &spec, &mut it) {
                Ok(k) => k,
                Err(e) => return err(e),
            },
        };
        if keep {
            kept.push(it);
        }
    }
    *items = kept;

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

pub fn ships(input: TokenStream) -> TokenStream {
    let tag = match syn::parse2::<syn::Ident>(input.clone()) {
        Ok(ident) => ident.to_string(),
        Err(_) => match syn::parse2::<LitStr>(input) {
            Ok(lit) => lit.value(),
            Err(e) => {
                return err(syn::Error::new(
                    e.span(),
                    "solex::ships! expects one tag, e.g. ships!(nightly)",
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
