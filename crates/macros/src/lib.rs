//! Proc macros for Solex. Everything here is resolved at compile time against
//! `Solex.yaml` and the `SOLEX_CHANNEL` environment variable, so the compiled
//! artifact contains exactly the features of one channel and nothing else.

use proc_macro::TokenStream;

mod context;
mod expand;

/// Ship an item only on the channels that carry the given feature tags.
///
/// ```ignore
/// #[channel(stable)]            // on `stable` and every channel including it
/// #[channel(nightly)]           // on `nightly` (and channels including it)
/// #[channel([nightly, 850])]    // on `nightly`, callable from epoch 850
/// #[channel(stable, [nightly, 850])]
/// ```
///
/// An item whose tags are not shipped by the active channel is removed from
/// the compilation entirely. An epoch bound injects a runtime check at the top
/// of the function body that fails with `solex::error::FEATURE_NOT_ACTIVE`
/// until the cluster reaches that epoch.
#[proc_macro_attribute]
pub fn channel(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand::channel(attr.into(), item.into()).into()
}

/// Program module wrapper. Applies `#[channel]` to every item of the module
/// before the framework's own `#[program]` sees it, so gated instructions never
/// reach the dispatch table. With solex's `anchor` feature the module is then
/// handed to `#[anchor_lang::program]`; with `native` it is emitted as-is.
#[proc_macro_attribute]
pub fn program(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand::program(attr.into(), item.into()).into()
}

/// Declare the program id of the active channel, read from `Solex.yaml`.
///
/// ```ignore
/// solex::declare_id!();
/// ```
#[proc_macro]
pub fn declare_id(input: TokenStream) -> TokenStream {
    expand::declare_id(input.into()).into()
}

/// Expands to the active channel name as a `&'static str`.
#[proc_macro]
pub fn active_channel(input: TokenStream) -> TokenStream {
    expand::active_channel(input.into()).into()
}

/// Process `#[channel(...)]` markers *inside* an item: on enum variants,
/// struct fields, match arms and statements, where attribute macros cannot be
/// applied directly. Excluded markers become `#[cfg(any())]`, included ones
/// are removed. Hand-rolled instruction enums and dispatch `match`es use this.
///
/// ```ignore
/// #[solex::gated]
/// pub enum Instruction {
///     Open = 1,
///     #[channel(nightly)]
///     OpenWithHooks = 10,
/// }
/// ```
#[proc_macro_attribute]
pub fn gated(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand::gated(attr.into(), item.into()).into()
}

/// Gate a handler behind a feature's activation epoch from the manifest.
///
/// ```ignore
/// #[cfg(feature = "hooks")]
/// #[solex::activation("hooks")]   // no-op unless [features.hooks] epoch names this channel
/// pub fn open_with_hooks(...) -> ProgramResult { ... }
/// ```
///
/// Where `[features.hooks] epoch = { mainnet = 850 }` names the active
/// channel, the function returns `FEATURE_NOT_ACTIVE` before epoch 850.
#[proc_macro_attribute]
pub fn activation(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand::activation(attr.into(), item.into()).into()
}

/// Expands to the value of a per-channel variable from the manifest, as a
/// `&'static str` literal. Fails to compile when the active channel does not
/// define it.
///
/// ```ignore
/// const TREASURY: [u8; 32] = decode_pubkey(solex::var!("treasury_owner"));
/// ```
#[proc_macro]
pub fn var(input: TokenStream) -> TokenStream {
    expand::var(input.into()).into()
}

/// Expands to the active cluster within the channel as a `&'static str`, or
/// `""` when the channel deploys identical bytes everywhere.
#[proc_macro]
pub fn active_cluster(input: TokenStream) -> TokenStream {
    expand::active_cluster(input.into()).into()
}

/// Expands to a `bool` literal: whether the active channel ships `tag`.
///
/// ```ignore
/// if solex::ships!(nightly) { ... }
/// ```
#[proc_macro]
pub fn ships(input: TokenStream) -> TokenStream {
    expand::ships(input.into()).into()
}

/// Implementation detail of `solex::__private::framework_program` for
/// framework-free builds: returns the module unchanged.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn passthrough_program(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Implementation detail of `solex::__private::framework_declare_id` for
/// framework-free builds.
#[doc(hidden)]
#[proc_macro]
pub fn native_declare_id(input: TokenStream) -> TokenStream {
    expand::native_declare_id(input.into()).into()
}
