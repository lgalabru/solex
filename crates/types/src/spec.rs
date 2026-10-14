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

/// The whole attribute: a non-empty list of channel entries, matched with
/// "any" semantics, plus an optional feature label written as a string:
/// `#[channel(devnet, "hooks")]`. The label names the feature the item
/// belongs to; `solex features` groups by it.
#[derive(Debug, Clone)]
pub struct FeatureSpec {
    pub entries: Vec<Entry>,
    pub feature: Option<String>,
}

impl FeatureSpec {
    /// The channel part of the attribute, as written: `devnet` or
    /// `stable, [nightly, 850]`.
    pub fn tags_text(&self) -> String {
        self.entries
            .iter()
            .map(|e| match e.epoch {
                Some(epoch) => format!("[{}, {epoch}]", e.tag),
                None => e.tag.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
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
    let ident: Ident = input.parse().map_err(|_| {
        input.error(
            "expected a channel tag such as `stable`, `nightly` or `[nightly, 850]`; a quoted \
             string is the feature name, e.g. #[channel(devnet, \"hooks\")]",
        )
    })?;
    Ok(ident.to_string())
}

/// One argument: a channel entry or the quoted feature name.
enum Arg {
    Entry(Entry),
    Feature(LitStr),
}

impl Parse for Arg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(LitStr) {
            Ok(Arg::Feature(input.parse()?))
        } else {
            Ok(Arg::Entry(input.parse()?))
        }
    }
}

impl Parse for FeatureSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(
                input.error("#[channel] needs at least one channel tag, e.g. #[channel(stable)]")
            );
        }
        let args: Punctuated<Arg, Token![,]> = Punctuated::parse_terminated(input)?;
        let mut entries = Vec::new();
        let mut feature: Option<LitStr> = None;
        for arg in args {
            match arg {
                Arg::Entry(e) => entries.push(e),
                Arg::Feature(lit) => {
                    if feature.is_some() {
                        return Err(syn::Error::new(
                            lit.span(),
                            "only one feature name per attribute",
                        ));
                    }
                    if lit.value().trim().is_empty() {
                        return Err(syn::Error::new(lit.span(), "feature name cannot be empty"));
                    }
                    feature = Some(lit);
                }
            }
        }
        if entries.is_empty() {
            return Err(input.error(
                "#[channel] needs a channel tag next to the feature name, e.g. \
                 #[channel(devnet, \"hooks\")]",
            ));
        }
        Ok(FeatureSpec {
            entries,
            feature: feature.map(|l| l.value()),
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
    fn parses_assignment_epoch_and_feature_name() {
        let spec = parse(quote!(nightly = 900, "hooks"));
        assert_eq!(spec.entries[0].tag, "nightly");
        assert_eq!(spec.entries[0].epoch, Some(900));
        assert_eq!(spec.feature.as_deref(), Some("hooks"));
        assert_eq!(spec.tags_text(), "[nightly, 900]");
        let spec = parse(quote!("hooks", devnet));
        assert_eq!(spec.feature.as_deref(), Some("hooks"));
        assert_eq!(spec.tags_text(), "devnet");
    }

    #[test]
    fn rejects_empty_and_malformed() {
        assert!(syn::parse2::<FeatureSpec>(quote!()).is_err());
        assert!(syn::parse2::<FeatureSpec>(quote!([nightly])).is_err());
        assert!(syn::parse2::<FeatureSpec>(quote!([nightly, 1, 2])).is_err());
        assert!(syn::parse2::<FeatureSpec>(quote!(42)).is_err());
        // A feature name alone says nothing about where it ships.
        assert!(syn::parse2::<FeatureSpec>(quote!("hooks")).is_err());
        assert!(syn::parse2::<FeatureSpec>(quote!(devnet, "a", "b")).is_err());
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
