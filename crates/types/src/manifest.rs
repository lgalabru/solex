//! Reading channel configuration from disk: a standalone `Solex.toml` /
//! `Solex.yaml`, or a `solex` section embedded in another tool's manifest.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Channels, Error, Result};

/// Environment variable selecting the channel a build targets.
pub const CHANNEL_ENV: &str = "SOLEX_CHANNEL";

/// Environment variable pointing at the manifest to read (standalone or host).
pub const MANIFEST_ENV: &str = "SOLEX_MANIFEST";

/// Key under which host manifests (Anchor.toml, Quasar.toml, ...) embed the
/// [`Channels`] section.
pub const SECTION_KEY: &str = "solex";

/// Standalone file names, in lookup order.
pub const MANIFEST_FILES: &[&str] = &["Solex.toml", "Solex.yaml", "Solex.yml"];

/// Host manifests searched for a `solex` section, in lookup order.
pub const HOST_FILES: &[&str] = &["Anchor.toml", "Quasar.toml"];

/// The standalone manifest: a [`Channels`] section plus project metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// Program name, shown by the CLI. Defaults to the Cargo package name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Path to the program crate, relative to the manifest. Defaults to `.`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_path: Option<PathBuf>,
    /// How to turn the program crate into a `.so`. Defaults to `cargo build-sbf`.
    #[serde(default, skip_serializing_if = "BuildConfig::is_default")]
    pub build: BuildConfig,
    #[serde(flatten)]
    pub channels: Channels,
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        self.channels.validate()
    }
}

/// The build pipeline. Solex only owns the environment (`SOLEX_CHANNEL`,
/// `SOLEX_MANIFEST`); the command can be anything that compiles the crate.
///
/// ```toml
/// [build]
/// # default: cargo build-sbf with --manifest-path and --sbf-out-dir added
/// args = ["--tools-version", "v1.58", "--arch", "v3"]
///
/// # upstream rustc + sbpf-linker (https://github.com/blueshift-gg/sbpf-linker)
/// # command = ["cargo", "+nightly", "build-bpf"]
/// # artifact = "target/bpfel-unknown-none/release/lib{lib}.so"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    /// Program and leading arguments, run in the program directory. Empty
    /// means `cargo build-sbf`, for which solex also passes `--manifest-path`
    /// and `--sbf-out-dir`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// Arguments appended after the command on every channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Where the command leaves the `.so`, relative to the project root, with
    /// `{name}` (package name), `{lib}` (package name with `-` as `_`) and
    /// `{channel}` substituted. Required for a custom `command`; the artifact
    /// is copied to `target/solex/<channel>/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

impl BuildConfig {
    pub fn is_default(&self) -> bool {
        self.command.is_empty() && self.args.is_empty() && self.artifact.is_none()
    }

    /// Whether the pipeline is the stock `cargo build-sbf`.
    pub fn is_build_sbf(&self) -> bool {
        self.command.is_empty()
            || (self.command.len() == 2
                && self.command[0] == "cargo"
                && self.command[1] == "build-sbf")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Toml,
    Yaml,
}

impl Format {
    pub fn from_path(path: &Path) -> Result<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("toml") => Ok(Format::Toml),
            Some("yaml") | Some("yml") => Ok(Format::Yaml),
            _ => Err(Error::UnsupportedFormat(path.to_path_buf())),
        }
    }
}

/// Where a configuration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A `Solex.toml` / `Solex.yaml` whose top level is a [`Manifest`].
    Standalone,
    /// A host document with a `solex` key holding a [`Manifest`] or [`Channels`].
    Embedded,
}

/// A configuration together with where it was read from.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub manifest: Manifest,
    pub path: PathBuf,
    pub format: Format,
    pub source: Source,
}

impl Loaded {
    /// Directory containing the manifest.
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

    pub fn channels(&self) -> &Channels {
        &self.manifest.channels
    }
}

/// Parse a document in `format`. Honors a top-level `solex` key when present
/// (host manifest), otherwise reads the whole document as a [`Manifest`].
pub fn parse(text: &str, path: &Path, format: Format) -> Result<(Manifest, Source)> {
    let parse_err = |e: &dyn std::fmt::Display| Error::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    let (manifest, source): (Manifest, Source) = match format {
        #[cfg(feature = "toml")]
        Format::Toml => {
            let doc: toml::Table = toml::from_str(text).map_err(|e| parse_err(&e))?;
            match doc.get(SECTION_KEY) {
                Some(section) => (
                    section
                        .clone()
                        .try_into::<Manifest>()
                        .map_err(|e| parse_err(&e))?,
                    Source::Embedded,
                ),
                None => (
                    doc.try_into::<Manifest>().map_err(|e| parse_err(&e))?,
                    Source::Standalone,
                ),
            }
        }
        #[cfg(feature = "yaml")]
        Format::Yaml => {
            let doc: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| parse_err(&e))?;
            match doc.get(SECTION_KEY) {
                Some(section) => (
                    serde_yaml::from_value(section.clone()).map_err(|e| parse_err(&e))?,
                    Source::Embedded,
                ),
                None => (
                    serde_yaml::from_value(doc).map_err(|e| parse_err(&e))?,
                    Source::Standalone,
                ),
            }
        }
        #[allow(unreachable_patterns)]
        other => {
            return Err(Error::FormatDisabled {
                feature: match other {
                    Format::Toml => "toml",
                    Format::Yaml => "yaml",
                },
                path: path.to_path_buf(),
            });
        }
    };
    manifest.validate()?;
    Ok((manifest, source))
}

/// Serialize a standalone manifest in `format`.
pub fn to_string(manifest: &Manifest, format: Format) -> Result<String> {
    let err = |e: &dyn std::fmt::Display| Error::Parse {
        path: PathBuf::from(match format {
            Format::Toml => "Solex.toml",
            Format::Yaml => "Solex.yaml",
        }),
        message: e.to_string(),
    };
    match format {
        #[cfg(feature = "toml")]
        Format::Toml => toml::to_string_pretty(manifest).map_err(|e| err(&e)),
        #[cfg(feature = "yaml")]
        Format::Yaml => serde_yaml::to_string(manifest).map_err(|e| err(&e)),
        #[allow(unreachable_patterns)]
        other => Err(Error::FormatDisabled {
            feature: match other {
                Format::Toml => "toml",
                Format::Yaml => "yaml",
            },
            path: PathBuf::new(),
        }),
    }
}

/// Read the configuration at an explicit path.
pub fn load(path: &Path) -> Result<Loaded> {
    let format = Format::from_path(path)?;
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let (manifest, source) = parse(&text, path, format)?;
    Ok(Loaded {
        manifest,
        path: path.to_path_buf(),
        format,
        source,
    })
}

/// Does this host document carry a `solex` section? Cheap textual check so
/// discovery can skip unrelated Anchor/Quasar projects without a full parse.
fn has_section(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| {
        text.lines().any(|l| {
            let l = l.trim();
            l.starts_with("[solex]")
                || l.starts_with("[solex.")
                || l.starts_with("solex:")
                || l.starts_with("solex =")
        })
    })
}

/// Locate the configuration: `SOLEX_MANIFEST` if set, else walk up from
/// `start` checking standalone files first, then host manifests with a
/// `solex` section.
pub fn find(start: &Path) -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var(MANIFEST_ENV) {
        let explicit = explicit.trim();
        if !explicit.is_empty() {
            return Ok(PathBuf::from(explicit));
        }
    }
    let mut dir = Some(start);
    while let Some(d) = dir {
        for name in MANIFEST_FILES {
            let candidate = d.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        for name in HOST_FILES {
            let candidate = d.join(name);
            if candidate.is_file() && has_section(&candidate) {
                return Ok(candidate);
            }
        }
        dir = d.parent();
    }
    Err(Error::NotFound(start.to_path_buf()))
}

/// [`find`] followed by [`load`].
pub fn discover(start: &Path) -> Result<Loaded> {
    load(&find(start)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
name: counter
default_channel: stable
build:
  args: ["--arch", "v3"]
channels:
  stable:
    program_id: "11111111111111111111111111111111"
    cluster: mainnet-beta
    vars:
      treasury: "4zTeC5mVqWLruDexgU2mV66p9t5vCA9JyiZqdGDUspap"
  nightly:
    includes: [stable]
    program_id: "22222222222222222222222222222222"
    cluster: devnet
"#;

    const TOML: &str = r#"
name = "counter"
default_channel = "stable"
build = { args = ["--arch", "v3"] }

[channels.stable]
program_id = "11111111111111111111111111111111"
cluster = "mainnet-beta"
vars = { treasury = "4zTeC5mVqWLruDexgU2mV66p9t5vCA9JyiZqdGDUspap" }

[channels.nightly]
includes = ["stable"]
program_id = "22222222222222222222222222222222"
cluster = "devnet"
"#;

    const ANCHOR_TOML: &str = r#"
[toolchain]
anchor_version = "1.0.0"

[programs.localnet]
counter = "11111111111111111111111111111111"

[solex]
default_channel = "stable"

[solex.channels.stable]
program_id = "11111111111111111111111111111111"

[solex.channels.nightly]
program_id = "22222222222222222222222222222222"
includes = ["stable"]
"#;

    #[test]
    fn yaml_and_toml_standalone_agree() {
        let (y, sy) = parse(YAML, Path::new("Solex.yaml"), Format::Yaml).unwrap();
        let (t, st) = parse(TOML, Path::new("Solex.toml"), Format::Toml).unwrap();
        assert_eq!(y, t);
        assert_eq!((sy, st), (Source::Standalone, Source::Standalone));
        assert_eq!(y.channels.names(), ["stable", "nightly"]);
        assert_eq!(y.name.as_deref(), Some("counter"));
        assert_eq!(y.build.args, ["--arch", "v3"]);
        assert!(y.build.is_build_sbf());
        assert_eq!(
            y.channels.channel("stable").unwrap().vars["treasury"],
            "4zTeC5mVqWLruDexgU2mV66p9t5vCA9JyiZqdGDUspap"
        );
        assert!(y.channels.channel("nightly").unwrap().vars.is_empty());
    }

    #[test]
    fn embedded_section_in_anchor_toml() {
        let (m, source) = parse(ANCHOR_TOML, Path::new("Anchor.toml"), Format::Toml).unwrap();
        assert_eq!(source, Source::Embedded);
        assert_eq!(m.name, None);
        assert_eq!(
            m.channels.program_id("nightly").unwrap(),
            "22222222222222222222222222222222"
        );
        assert!(m.channels.ships("nightly", "stable").unwrap());
    }

    #[test]
    fn embedded_section_in_yaml() {
        let doc = format!("something_else: 1\nsolex:\n{}", indent(YAML));
        let (m, source) = parse(&doc, Path::new("Host.yaml"), Format::Yaml).unwrap();
        assert_eq!(source, Source::Embedded);
        assert_eq!(m.channels.names(), ["stable", "nightly"]);
    }

    fn indent(s: &str) -> String {
        s.lines()
            .map(|l| {
                if l.is_empty() {
                    l.to_string()
                } else {
                    format!("  {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn roundtrip_both_formats() {
        let (m, _) = parse(YAML, Path::new("Solex.yaml"), Format::Yaml).unwrap();
        for f in [Format::Yaml, Format::Toml] {
            let text = to_string(&m, f).unwrap();
            let (again, _) = parse(&text, Path::new("x"), f).unwrap();
            assert_eq!(m, again, "{f:?}");
        }
    }

    #[test]
    fn rejects_unknown_extension_and_bad_docs() {
        assert!(matches!(
            Format::from_path(Path::new("Solex.json")),
            Err(Error::UnsupportedFormat(_))
        ));
        assert!(parse("channels = 3", Path::new("x.toml"), Format::Toml).is_err());
        assert!(parse("name: x\nchannels: {}", Path::new("x.yaml"), Format::Yaml).is_err());
    }

    #[test]
    fn find_prefers_standalone_then_host_with_section() {
        if std::env::var(MANIFEST_ENV).is_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("programs").join("counter").join("src");
        std::fs::create_dir_all(&nested).unwrap();

        // Anchor.toml without a solex section is skipped...
        std::fs::write(dir.path().join("Anchor.toml"), "[toolchain]\n").unwrap();
        assert!(matches!(find(&nested), Err(Error::NotFound(_))));

        // ...with one, it is found.
        std::fs::write(dir.path().join("Anchor.toml"), ANCHOR_TOML).unwrap();
        assert_eq!(find(&nested).unwrap(), dir.path().join("Anchor.toml"));
        let loaded = discover(&nested).unwrap();
        assert_eq!(loaded.source, Source::Embedded);
        assert_eq!(loaded.format, Format::Toml);

        // A standalone file wins over the host manifest.
        std::fs::write(dir.path().join("Solex.toml"), TOML).unwrap();
        assert_eq!(find(&nested).unwrap(), dir.path().join("Solex.toml"));
    }
}
