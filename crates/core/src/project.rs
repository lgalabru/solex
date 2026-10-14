//! A Solex project on disk: the channel configuration plus derived paths.
//!
//! Framework CLIs that already parsed their own manifest (Anchor.toml,
//! Quasar.toml) build a `Project` with [`Project::from_parts`] and reuse the
//! build, deploy and scan logic without going through discovery.

use std::path::{Path, PathBuf};

use solex_types::{Channels, Format, Loaded, Manifest, Source};

use crate::{CoreError, Result};

#[derive(Debug, Clone)]
pub struct Project {
    pub loaded: Loaded,
}

impl Project {
    /// Walk up from `start` to find `Solex.toml`, `Solex.yaml`, or a host
    /// manifest with a `solex` section.
    pub fn discover(start: &Path) -> Result<Self> {
        Ok(Self {
            loaded: solex_types::discover(start)?,
        })
    }

    pub fn discover_cwd() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|e| CoreError::io("read", ".", e))?;
        Self::discover(&cwd)
    }

    /// Read a specific manifest file (standalone or host).
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self {
            loaded: solex_types::load(path)?,
        })
    }

    /// Build a project from an already-parsed `solex` section.
    ///
    /// `manifest_path` is the file the section came from; it is handed to the
    /// proc macros through `SOLEX_MANIFEST` so compile-time resolution sees the
    /// same configuration the CLI did.
    pub fn from_parts(
        channels: Channels,
        manifest_path: &Path,
        program_path: Option<PathBuf>,
        name: Option<String>,
    ) -> Result<Self> {
        channels.validate()?;
        let format = Format::from_path(manifest_path)?;
        Ok(Self {
            loaded: Loaded {
                manifest: Manifest {
                    name,
                    program_path,
                    channels,
                },
                path: manifest_path.to_path_buf(),
                format,
                source: Source::Embedded,
            },
        })
    }

    pub fn channels(&self) -> &Channels {
        &self.loaded.manifest.channels
    }

    pub fn manifest(&self) -> &Manifest {
        &self.loaded.manifest
    }

    pub fn manifest_path(&self) -> &Path {
        &self.loaded.path
    }

    /// Directory containing the manifest.
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

    /// Program name: `name` from the manifest, else the Cargo package name,
    /// else the directory name.
    pub fn name(&self) -> String {
        if let Some(n) = &self.loaded.manifest.name {
            return n.clone();
        }
        if let Some(n) = cargo_package_name(&self.program_manifest()) {
            return n;
        }
        self.program_dir()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "program".into())
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
        let ch = self.channels().channel(channel)?;
        Ok(match &ch.keypair {
            Some(p) => self.resolve_path(p),
            None => self.root().join("keys").join(format!("{channel}.json")),
        })
    }

    /// Persist a standalone manifest back to disk. Embedded sections belong to
    /// the host tool's manifest and are not rewritten here.
    pub fn save(&self) -> Result<()> {
        if self.loaded.source == Source::Embedded {
            return Err(CoreError::message(format!(
                "the solex section lives in {}; edit it there",
                self.loaded.path.display()
            )));
        }
        let text = solex_types::to_string(self.manifest(), self.loaded.format)?;
        std::fs::write(&self.loaded.path, text)
            .map_err(|e| CoreError::io("write", self.loaded.path.clone(), e))
    }
}

fn cargo_package_name(cargo_toml: &Path) -> Option<String> {
    let text = std::fs::read_to_string(cargo_toml).ok()?;
    let doc: toml::Table = toml::from_str(&text).ok()?;
    doc.get("package")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_parts_and_name_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-counter\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let anchor = dir.path().join("Anchor.toml");
        std::fs::write(&anchor, "[solex]\n").unwrap();
        let mut channels = Channels::default();
        channels.channels.insert(
            "stable".into(),
            solex_types::Channel {
                program_id: Some("1".into()),
                ..Default::default()
            },
        );
        let p = Project::from_parts(channels, &anchor, None, None).unwrap();
        assert_eq!(p.name(), "my-counter");
        assert_eq!(p.channels().resolve(Some("stable")).unwrap(), "stable");
        assert!(p.save().is_err(), "embedded sections are never rewritten");
    }
}
