//! Grammar of `#[channel(...)]` and the pure decision of whether an item ships.
//!
//! Shared by the proc macros (compile-time gating) and source scanners such as
//! `solex features`, so both agree on what an attribute means.

use std::collections::BTreeSet;

use proc_macro2::Span;
use syn::{
    Ident, LitInt, LitStr, Token, bracketed,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

/// One `tag` or `[tag, epoch]` entry.
#[derive(Debug, Clone)]
pub struct Entry {
    pub tag: String,
    pub epoch: Option<u64>,
    pub span: Span,
}

/// The whole attribute: a non-empty list of entries, matched with "any" semantics.
#[derive(Debug, Clone)]
pub struct FeatureSpec {
    pub entries: Vec<Entry>,
}

impl Parse for Entry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(syn::token::Bracket) {
            let content;
            let bracket = bracketed!(content in input);
            let tag = parse_tag(&content)?;
            content.parse::<Token![,]>().map_err(|_| {
                syn::Error::new(
                    bracket.span.join(),
                    "expected `[tag, epoch]`, e.g. `[nightly, 850]`",
                )
            })?;
            let epoch: LitInt = content.parse()?;
            let epoch = epoch.base10_parse::<u64>()?;
            if !content.is_empty() {
                return Err(content.error("expected `[tag, epoch]` with exactly two elements"));
            }
            return Ok(Entry {
                tag,
                epoch: Some(epoch),
                span: bracket.span.join(),
            });
        }
        let span = input.span();
        let tag = parse_tag(input)?;
        // `tag = epoch` is accepted as a flatter spelling of `[tag, epoch]`.
        let epoch = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse::<LitInt>()?.base10_parse::<u64>()?)
        } else {
            None
        };
        Ok(Entry { tag, epoch, span })
    }
}

fn parse_tag(input: ParseStream) -> syn::Result<String> {
    if input.peek(LitStr) {
        return Ok(input.parse::<LitStr>()?.value());
    }
    let ident: Ident = input.parse().map_err(|_| {
        input.error("expected a channel tag such as `stable`, `nightly` or `[nightly, 850]`")
    })?;
    Ok(ident.to_string())
}

impl Parse for FeatureSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(
                input.error("#[channel] needs at least one channel tag, e.g. #[channel(stable)]")
            );
        }
        let entries: Punctuated<Entry, Token![,]> = Punctuated::parse_terminated(input)?;
        Ok(FeatureSpec {
            entries: entries.into_iter().collect(),
        })
    }
}

/// Outcome of matching a spec against the active channel's tag set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The item is dropped from this build.
    Exclude,
    /// The item ships, callable once the cluster reaches `epoch` (if any).
    Include { epoch: Option<u64> },
}

/// "Any" semantics across entries. If one matching entry has no epoch bound the
/// item is unconditionally active; otherwise the earliest epoch wins.
pub fn decide(spec: &FeatureSpec, tags: &BTreeSet<String>) -> Decision {
    let mut best: Option<Option<u64>> = None;
    for entry in &spec.entries {
        if !tags.contains(&entry.tag) {
            continue;
        }
        best = Some(match (best, entry.epoch) {
            (Some(None), _) | (_, None) => None,
            (Some(Some(a)), Some(b)) => Some(a.min(b)),
            (None, Some(b)) => Some(b),
        });
    }
    match best {
        None => Decision::Exclude,
        Some(epoch) => Decision::Include { epoch },
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    fn tags(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn parse(ts: proc_macro2::TokenStream) -> FeatureSpec {
        syn::parse2(ts).expect("spec parses")
    }

    #[test]
    fn parses_bare_tags() {
        let spec = parse(quote!(stable, nightly));
        let got: Vec<_> = spec
            .entries
            .iter()
            .map(|e| (e.tag.as_str(), e.epoch))
            .collect();
        assert_eq!(got, [("stable", None), ("nightly", None)]);
    }

    #[test]
    fn parses_bracketed_epoch() {
        let spec = parse(quote!([nightly, 850]));
        assert_eq!(spec.entries[0].tag, "nightly");
        assert_eq!(spec.entries[0].epoch, Some(850));
    }

    #[test]
    fn parses_assignment_epoch_and_strings() {
        let spec = parse(quote!("stable", nightly = 900));
        assert_eq!(spec.entries[0].tag, "stable");
        assert_eq!(spec.entries[1].epoch, Some(900));
    }

    #[test]
    fn rejects_empty_and_malformed() {
        assert!(syn::parse2::<FeatureSpec>(quote!()).is_err());
        assert!(syn::parse2::<FeatureSpec>(quote!([nightly])).is_err());
        assert!(syn::parse2::<FeatureSpec>(quote!([nightly, 1, 2])).is_err());
        assert!(syn::parse2::<FeatureSpec>(quote!(42)).is_err());
    }

    #[test]
    fn decide_excludes_when_no_tag_matches() {
        let spec = parse(quote!(nightly));
        assert_eq!(decide(&spec, &tags(&["stable"])), Decision::Exclude);
    }

    #[test]
    fn decide_includes_without_epoch_when_any_match_is_unbounded() {
        let spec = parse(quote!(stable, [nightly, 850]));
        assert_eq!(
            decide(&spec, &tags(&["stable", "nightly"])),
            Decision::Include { epoch: None }
        );
    }

    #[test]
    fn decide_keeps_earliest_epoch() {
        let spec = parse(quote!([nightly, 900], [canary, 850]));
        assert_eq!(
            decide(&spec, &tags(&["nightly", "canary"])),
            Decision::Include { epoch: Some(850) }
        );
        assert_eq!(
            decide(&spec, &tags(&["nightly"])),
            Decision::Include { epoch: Some(900) }
        );
    }
}
