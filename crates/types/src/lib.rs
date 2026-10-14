//! Types for Solex release channels.
//!
//! This crate is the integration point for other tools. It has three layers:
//!
//! - [`Channels`]: the embeddable config section (`default_channel` plus a map
//!   of channels). This is what a framework manifest carries under a `solex`
//!   key, and it owns all resolution logic (tag sets, `includes`, which channel
//!   a build targets).
//! - [`Manifest`]: the standalone `Solex.yaml` / `Solex.toml` file, which is a
//!   [`Channels`] section plus `name` and `program_path`.
//! - [`load`] / [`discover`]: read either form from disk, sniffing the format
//!   by extension and accepting any YAML or TOML document that has a top-level
//!   `solex` key.
//!
//! The `spec` feature adds [`spec`], the grammar of `#[channel(...)]`, shared by
//! the proc macros and by `solex features`.
//!
//! No Solana dependencies live here, so the crate is cheap to pull into a CLI.

mod channels;
mod manifest;
#[cfg(feature = "spec")]
pub mod spec;

pub use channels::{Channel, Channels, is_valid_channel_name};
pub use manifest::{
    CHANNEL_ENV, Format, HOST_FILES, Loaded, MANIFEST_ENV, MANIFEST_FILES, Manifest, SECTION_KEY,
    Source, discover, find, load, parse, to_string,
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Solex configuration found (searched upward from {})\n  Looked for {} or a `{SECTION_KEY}` section in Anchor.toml / Quasar.toml", .0.display(), MANIFEST_FILES.join(", "))]
    NotFound(std::path::PathBuf),
    #[error("failed to read {}: {source}", path.display())]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid {}: {message}", path.display())]
    Parse {
        path: std::path::PathBuf,
        message: String,
    },
    #[error("{} has no `{SECTION_KEY}` section", .0.display())]
    MissingSection(std::path::PathBuf),
    #[error("unsupported manifest format for {} (expected .toml, .yaml or .yml)", .0.display())]
    UnsupportedFormat(std::path::PathBuf),
    #[error("the `{feature}` feature of solex-types is required to read {}", path.display())]
    FormatDisabled {
        feature: &'static str,
        path: std::path::PathBuf,
    },
    #[error("unknown channel `{name}`; defined channels: {}", defined.join(", "))]
    UnknownChannel { name: String, defined: Vec<String> },
    #[error("channel `{channel}` includes unknown channel `{included}`")]
    UnknownInclude { channel: String, included: String },
    #[error("channel include cycle: {}", path.join(" -> "))]
    IncludeCycle { path: Vec<String> },
    #[error("no channels defined; add at least one entry under `channels`")]
    NoChannels,
    #[error(
        "no channel selected; set {CHANNEL_ENV}, pass --channel, or set `default_channel` \
         (channels: {})",
        defined.join(", ")
    )]
    NoDefault { defined: Vec<String> },
    #[error("channel `{0}` has no `program_id`")]
    MissingProgramId(String),
    #[error("invalid channel name `{0}`: use lowercase letters, digits, `-` or `_`")]
    InvalidChannelName(String),
}
