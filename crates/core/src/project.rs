//! A Solex project on disk: the manifest plus derived paths.

use std::path::{Path, PathBuf};

use solex_manifest::{LoadedManifest, Manifest};

use crate::{CoreError, Result};

#[derive(Debug, Clone)]
pub struct Project {
    pub loaded: LoadedManifest,
}

impl Project {
    /// Walk up from `start` to find `Solex.yaml`.
    pub fn discover(start: &Path) -> Result<Self> {
        Ok(Self {
            loaded: solex_manifest::discover(start)?,
        })
    }

    pub fn discover_cwd() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|e| CoreError::io("read", ".", e))?;
        Self::discover(&cwd)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.loaded.manifest
    }

    pub fn manifest_path(&self) -> &Path {
        &self.loaded.path
    }

    /// Directory containing `Solex.yaml`.
    pub fn root(&self) -> &Path {
        self.loaded.root()
    }

    /// Directory of the program crate (`program_path` or the root).
    pub fn program_dir(&self) -> PathBuf {
        self.loaded.program_dir()
    }

    pub fn program_manifest(&self) -> PathBuf {
        self.program_dir().join("Cargo.toml")
    }

    /// Where `solex build` puts artifacts for a channel.
    pub fn channel_out_dir(&self, channel: &str) -> PathBuf {
        self.root().join("target").join("solex").join(channel)
    }

    /// Resolve a path from the manifest relative to the project root, expanding
    /// a leading `~`.
    pub fn resolve_path(&self, p: &Path) -> PathBuf {
        let expanded = expand_home(p);
        if expanded.is_absolute() {
            expanded
        } else {
            self.root().join(expanded)
        }
    }

    /// Keypair path for a channel, defaulting to `keys/<channel>.json`.
    pub fn channel_keypair(&self, channel: &str) -> Result<PathBuf> {
        let ch = self.manifest().channel(channel)?;
        Ok(match &ch.keypair {
            Some(p) => self.resolve_path(p),
            None => self.root().join("keys").join(format!("{channel}.json")),
        })
    }

    /// Persist the manifest back to disk (used by `init` and `keys new`).
    pub fn save(&self) -> Result<()> {
        std::fs::write(&self.loaded.path, self.manifest().to_yaml())
            .map_err(|e| CoreError::io("write", self.loaded.path.clone(), e))
    }
}

pub fn expand_home(p: &Path) -> PathBuf {
    let Some(s) = p.to_str() else {
        return p.to_path_buf();
    };
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    p.to_path_buf()
}
