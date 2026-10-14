//! Compile-time view of the project: the manifest next to (or above) the crate
//! being compiled, and the channel selected for this build.

use std::{collections::BTreeSet, path::PathBuf};

use proc_macro2::Span;
use solex_types::{CHANNEL_ENV, Loaded, MANIFEST_ENV};

pub struct BuildContext {
    pub loaded: Loaded,
    pub channel: String,
    pub tags: BTreeSet<String>,
}

impl BuildContext {
    /// Discover the manifest from `CARGO_MANIFEST_DIR` and resolve the channel
    /// from `SOLEX_CHANNEL` or the manifest default.
    pub fn load(span: Span) -> syn::Result<Self> {
        let start = std::env::var_os("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let loaded = solex_types::discover(&start).map_err(|e| {
            syn::Error::new(
                span,
                format!(
                    "solex: {e}\n\nSolex macros read the channel configuration at compile time. \
                     Create one with `solex init`, add a [solex] section to Anchor.toml, or set \
                     {MANIFEST_ENV} to its path."
                ),
            )
        })?;
        let channel = loaded.channels().resolve(None).map_err(|e| {
            syn::Error::new(
                span,
                format!("solex: {e}\n\nBuild with `solex build --channel <name>` or export {CHANNEL_ENV}."),
            )
        })?;
        let tags = loaded
            .channels()
            .tags(&channel)
            .map_err(|e| syn::Error::new(span, format!("solex: {e}")))?;
        Ok(Self {
            loaded,
            channel,
            tags,
        })
    }

    pub fn program_id(&self, span: Span) -> syn::Result<String> {
        let id = self
            .loaded
            .channels()
            .program_id(&self.channel)
            .map_err(|e| syn::Error::new(span, format!("solex: {e}")))?;
        validate_pubkey(id).map_err(|msg| {
            syn::Error::new(
                span,
                format!(
                    "solex: channel `{}` has an invalid program_id `{id}`: {msg}",
                    self.channel
                ),
            )
        })?;
        Ok(id.to_string())
    }
}

/// A program id must be 32 bytes of base58.
pub fn validate_pubkey(s: &str) -> Result<(), String> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| format!("not base58 ({e})"))?;
    if bytes.len() != 32 {
        return Err(format!("decodes to {} bytes, expected 32", bytes.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_pubkey;

    #[test]
    fn accepts_system_program_and_rejects_garbage() {
        assert!(validate_pubkey("11111111111111111111111111111111").is_ok());
        assert!(validate_pubkey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").is_ok());
        assert!(validate_pubkey("not-a-key").is_err());
        assert!(validate_pubkey("1111").is_err());
    }
}
