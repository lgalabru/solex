//! Core logic behind the `solex` CLI: locating a project, generating channel
//! keypairs, scanning sources for `#[channel]` attributes, and driving
//! `cargo build-sbf` / `solana program deploy` per channel.
//!
//! No terminal styling lives here; the CLI crate renders results.

use std::path::PathBuf;

pub mod build;
pub mod close;
pub mod deploy;
pub mod keys;
pub mod project;
pub mod scaffold;
pub mod scan;

pub use project::Project;
pub use solex_types::{self as types, BuildConfig, Channel, Channels, Manifest};

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Types(#[from] solex_types::Error),
    #[error("failed to {action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid keypair file {}: {reason}", path.display())]
    Keypair { path: PathBuf, reason: String },
    #[error("failed to parse {}: {source}", path.display())]
    Syntax {
        path: PathBuf,
        #[source]
        source: syn::Error,
    },
    #[error("failed to run `{program}`: {source}\n  Is it installed and on PATH?")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{command} failed with exit code {code}\n{output}")]
    CommandFailed {
        command: String,
        code: i32,
        output: String,
    },
    #[error("{0}")]
    Message(String),
}

impl CoreError {
    pub fn io(action: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }

    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }

    /// Exit code the CLI should use for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::CommandFailed { code, .. } if *code != 0 => *code,
            _ => 1,
        }
    }
}
