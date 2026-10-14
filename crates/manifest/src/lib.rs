//! `Solex.yaml` manifest: release channels, program addresses, and the rules
//! deciding which `#[channel(...)]` items ship on which channel.
//!
//! This crate is shared by the proc macros (which read the manifest at
//! compile time) and the CLI (which reads it to build and deploy). It has no
//! Solana dependencies so it stays cheap to compile on the host.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub mod spec;

/// File name looked up when locating a project manifest.
pub const MANIFEST_FILE: &str = "Solex.yaml";

/// Environment variable selecting the channel a build targets.
pub const CHANNEL_ENV: &str = "SOLEX_CHANNEL";

/// Environment variable overriding the manifest location.
pub const MANIFEST_ENV: &str = "SOLEX_MANIFEST";

pub type Result<T> = std::result::Result<T, ManifestError>;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("{MANIFEST_FILE} not found (searched from {})", .0.display())]
    NotFound(PathBuf),
    #[error("failed to read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("unknown channel `{name}`; defined channels: {}", defined.join(", "))]
    UnknownChannel { name: String, defined: Vec<String> },
    #[error("channel `{channel}` includes unknown channel `{included}`")]
    UnknownInclude { channel: String, included: String },
    #[error("channel include cycle: {}", path.join(" -> "))]
    IncludeCycle { path: Vec<String> },
    #[error("no channels defined; add at least one entry under `channels:`")]
    NoChannels,
    #[error(
        "no channel selected; set {CHANNEL_ENV}, pass --channel, or add `default_channel:` to \
         {MANIFEST_FILE} (channels: {})",
        defined.join(", ")
    )]
    NoDefault { defined: Vec<String> },
    #[error("channel `{0}` has no `program_id`")]
    MissingProgramId(String),
    #[error("invalid channel name `{0}`: use lowercase letters, digits, `-` or `_`")]
    InvalidChannelName(String),
}

/// Root of `Solex.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Program crate name (the Cargo package that gets built).
    pub name: String,
    /// Channel used when `SOLEX_CHANNEL` is unset and `--channel` is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_channel: Option<String>,
    /// Path to the program crate, relative to the manifest. Defaults to `.`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_path: Option<PathBuf>,
    /// Release channels keyed by name, in declaration order (the order is
    /// what `solex channels` and `solex features` display).
    pub channels: IndexMap<String, Channel>,
}

/// One release channel: a deployment target plus the set of feature tags it
/// ships.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    /// Address the program is deployed at on this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Keypair file for `program_id`; required for the first deploy only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypair: Option<PathBuf>,
    /// Cluster moniker (`mainnet-beta`, `devnet`, `testnet`, `localnet`) or
    /// RPC URL passed to the Solana CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
    /// Upgrade authority keypair passed to `solana program deploy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_authority: Option<PathBuf>,
    /// Other channels whose features this channel also ships. A `nightly`
    /// channel that `includes: [stable]` compiles every `#[channel(stable)]`
    /// item in addition to `#[channel(nightly)]` ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// Free-form description shown by `solex channels`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A manifest together with the path it was loaded from.
#[derive(Debug, Clone)]
pub struct LoadedManifest {
    pub manifest: Manifest,
    pub path: PathBuf,
}

impl LoadedManifest {
    /// Directory containing `Solex.yaml`.
    pub fn root(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    /// Absolute path of the program crate.
    pub fn program_dir(&self) -> PathBuf {
        match &self.manifest.program_path {
            Some(p) => self.root().join(p),
            None => self.root().to_path_buf(),
        }
    }
}

impl Manifest {
    pub fn parse(yaml: &str, path: &Path) -> Result<Self> {
        let manifest: Manifest =
            serde_yaml::from_str(yaml).map_err(|source| ManifestError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).unwrap_or_default()
    }

    /// Structural checks that do not need the environment: non-empty, valid
    /// names, includes resolve, no include cycles.
    pub fn validate(&self) -> Result<()> {
        if self.channels.is_empty() {
            return Err(ManifestError::NoChannels);
        }
        for (name, channel) in &self.channels {
            if !is_valid_channel_name(name) {
                return Err(ManifestError::InvalidChannelName(name.clone()));
            }
            for inc in &channel.includes {
                if !self.channels.contains_key(inc) {
                    return Err(ManifestError::UnknownInclude {
                        channel: name.clone(),
                        included: inc.clone(),
                    });
                }
            }
        }
        if let Some(def) = &self.default_channel {
            self.channel(def)?;
        }
        for name in self.channels.keys() {
            // `tags` walks the include graph and reports cycles.
            self.tags(name)?;
        }
        Ok(())
    }

    pub fn channel_names(&self) -> Vec<String> {
        self.channels.keys().cloned().collect()
    }

    pub fn channel(&self, name: &str) -> Result<&Channel> {
        self.channels
            .get(name)
            .ok_or_else(|| ManifestError::UnknownChannel {
                name: name.to_string(),
                defined: self.channel_names(),
            })
    }

    /// Program id for a channel, as a base58 string.
    pub fn program_id(&self, name: &str) -> Result<&str> {
        self.channel(name)?
            .program_id
            .as_deref()
            .ok_or_else(|| ManifestError::MissingProgramId(name.to_string()))
    }

    /// The full set of feature tags a channel ships: itself plus everything it
    /// includes, transitively.
    pub fn tags(&self, name: &str) -> Result<BTreeSet<String>> {
        let mut tags = BTreeSet::new();
        let mut stack = vec![name.to_string()];
        self.collect_tags(name, &mut tags, &mut stack)?;
        Ok(tags)
    }

    fn collect_tags(
        &self,
        name: &str,
        tags: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        let channel = self.channel(name)?;
        tags.insert(name.to_string());
        for inc in &channel.includes {
            if stack.iter().any(|s| s == inc) {
                let mut cycle = stack.clone();
                cycle.push(inc.clone());
                return Err(ManifestError::IncludeCycle { path: cycle });
            }
            if tags.contains(inc) {
                continue;
            }
            stack.push(inc.clone());
            self.collect_tags(inc, tags, stack)?;
            stack.pop();
        }
        Ok(())
    }

    /// Whether a build of `channel` ships items tagged `tag`.
    pub fn ships(&self, channel: &str, tag: &str) -> Result<bool> {
        Ok(self.tags(channel)?.contains(tag))
    }

    /// Resolve the channel to build: an explicit request, else the
    /// environment, else `default_channel`, else the only channel defined.
    pub fn resolve_channel(&self, requested: Option<&str>) -> Result<String> {
        if let Some(name) = requested {
            self.channel(name)?;
            return Ok(name.to_string());
        }
        if let Ok(name) = std::env::var(CHANNEL_ENV) {
            let name = name.trim();
            if !name.is_empty() {
                self.channel(name)?;
                return Ok(name.to_string());
            }
        }
        if let Some(def) = &self.default_channel {
            return Ok(def.clone());
        }
        if self.channels.len() == 1 {
            return Ok(self.channels.keys().next().cloned().unwrap_or_default());
        }
        Err(ManifestError::NoDefault {
            defined: self.channel_names(),
        })
    }
}

pub fn is_valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Load the manifest at an explicit path.
pub fn load(path: &Path) -> Result<LoadedManifest> {
    let yaml = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let manifest = Manifest::parse(&yaml, path)?;
    Ok(LoadedManifest {
        manifest,
        path: path.to_path_buf(),
    })
}

/// Locate `Solex.yaml` by honoring `SOLEX_MANIFEST`, then walking up from
/// `start` until a manifest is found.
pub fn find(start: &Path) -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var(MANIFEST_ENV) {
        let explicit = explicit.trim();
        if !explicit.is_empty() {
            return Ok(PathBuf::from(explicit));
        }
    }
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(MANIFEST_FILE);
        if candidate.is_file() {
            return Ok(candidate);
        }
        dir = d.parent();
    }
    Err(ManifestError::NotFound(start.to_path_buf()))
}

/// `find` followed by `load`.
pub fn discover(start: &Path) -> Result<LoadedManifest> {
    load(&find(start)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
name: counter
default_channel: stable
channels:
  stable:
    program_id: "11111111111111111111111111111111"
    cluster: mainnet-beta
  nightly:
    includes: [stable]
    program_id: "22222222222222222222222222222222"
    cluster: devnet
  canary:
    includes: [nightly]
    program_id: "33333333333333333333333333333333"
"#;

    fn sample() -> Manifest {
        Manifest::parse(SAMPLE, Path::new("Solex.yaml")).expect("sample parses")
    }

    #[test]
    fn tags_follow_includes_transitively() {
        let m = sample();
        assert_eq!(
            m.tags("stable").unwrap().into_iter().collect::<Vec<_>>(),
            ["stable"]
        );
        assert_eq!(
            m.tags("nightly").unwrap().into_iter().collect::<Vec<_>>(),
            ["nightly", "stable"]
        );
        assert_eq!(
            m.tags("canary").unwrap().into_iter().collect::<Vec<_>>(),
            ["canary", "nightly", "stable"]
        );
    }

    #[test]
    fn ships_is_directional() {
        let m = sample();
        assert!(m.ships("nightly", "stable").unwrap());
        assert!(!m.ships("stable", "nightly").unwrap());
    }

    #[test]
    fn include_cycles_are_rejected() {
        let yaml = r#"
name: x
channels:
  a: { includes: [b], program_id: "1" }
  b: { includes: [a], program_id: "1" }
"#;
        let err = Manifest::parse(yaml, Path::new("Solex.yaml")).unwrap_err();
        assert!(matches!(err, ManifestError::IncludeCycle { .. }), "{err}");
    }

    #[test]
    fn unknown_include_is_rejected() {
        let yaml = r#"
name: x
channels:
  a: { includes: [ghost], program_id: "1" }
"#;
        let err = Manifest::parse(yaml, Path::new("Solex.yaml")).unwrap_err();
        assert!(matches!(err, ManifestError::UnknownInclude { .. }), "{err}");
    }

    #[test]
    fn unknown_default_is_rejected() {
        let yaml = r#"
name: x
default_channel: nope
channels:
  a: { program_id: "1" }
"#;
        let err = Manifest::parse(yaml, Path::new("Solex.yaml")).unwrap_err();
        assert!(matches!(err, ManifestError::UnknownChannel { .. }), "{err}");
    }

    #[test]
    fn resolve_prefers_explicit_then_default() {
        let m = sample();
        assert_eq!(m.resolve_channel(Some("nightly")).unwrap(), "nightly");
        assert!(m.resolve_channel(Some("ghost")).is_err());
        // Tests run in parallel; only assert on the default when the env var is unset.
        if std::env::var(CHANNEL_ENV).is_err() {
            assert_eq!(m.resolve_channel(None).unwrap(), "stable");
        }
    }

    #[test]
    fn single_channel_needs_no_default() {
        let yaml = r#"
name: x
channels:
  only: { program_id: "1" }
"#;
        let m = Manifest::parse(yaml, Path::new("Solex.yaml")).unwrap();
        if std::env::var(CHANNEL_ENV).is_err() {
            assert_eq!(m.resolve_channel(None).unwrap(), "only");
        }
    }

    #[test]
    fn find_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST_FILE), SAMPLE).unwrap();
        let nested = dir.path().join("programs").join("counter").join("src");
        std::fs::create_dir_all(&nested).unwrap();
        if std::env::var(MANIFEST_ENV).is_err() {
            let found = find(&nested).unwrap();
            assert_eq!(found, dir.path().join(MANIFEST_FILE));
        }
    }

    #[test]
    fn channel_order_follows_declaration() {
        assert_eq!(sample().channel_names(), ["stable", "nightly", "canary"]);
    }

    #[test]
    fn yaml_roundtrip_is_stable() {
        let m = sample();
        let again = Manifest::parse(&m.to_yaml(), Path::new("Solex.yaml")).unwrap();
        assert_eq!(m, again);
    }
}
