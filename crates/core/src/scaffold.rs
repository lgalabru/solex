//! `solex init`: a minimal Anchor program wired to Solex with one keypair per
//! channel.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use solex_types::{Channel, Channels, Feature, Format, Manifest, is_valid_channel_name};

use crate::{CoreError, Result, keys::Keypair};

#[derive(Debug, Clone)]
pub struct InitOptions {
    pub name: String,
    /// Channels in order; the first is the default and later ones include
    /// their predecessor (`canary` includes `stable`, ...).
    pub channels: Vec<String>,
    /// Cluster per channel; falls back to `localnet`.
    pub clusters: BTreeMap<String, String>,
    pub solex_version: String,
    /// `Solex.yaml` or `Solex.toml`.
    pub format: Format,
}

impl InitOptions {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || !self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CoreError::message(format!(
                "invalid project name `{}`: use letters, digits, `-` or `_`",
                self.name
            )));
        }
        if self.channels.is_empty() {
            return Err(CoreError::message("at least one channel is required"));
        }
        for ch in &self.channels {
            if !is_valid_channel_name(ch) {
                return Err(CoreError::message(format!(
                    "invalid channel name `{ch}`: use lowercase letters, digits, `-` or `_`"
                )));
            }
        }
        Ok(())
    }
}

pub struct Created {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
    /// `(channel, program_id)` in channel order.
    pub program_ids: Vec<(String, String)>,
}

pub fn init(parent: &Path, opts: &InitOptions) -> Result<Created> {
    opts.validate()?;
    let root = parent.join(&opts.name);
    if root.exists() {
        return Err(CoreError::message(format!(
            "{} already exists",
            root.display()
        )));
    }
    std::fs::create_dir_all(root.join("src")).map_err(|e| CoreError::io("create", &root, e))?;

    let mut files = Vec::new();
    let mut program_ids = Vec::new();
    let mut channels = IndexMap::new();
    for (i, name) in opts.channels.iter().enumerate() {
        let kp = Keypair::generate();
        let rel = PathBuf::from("keys").join(format!("{name}.json"));
        kp.write(&root.join(&rel))?;
        files.push(rel.clone());
        program_ids.push((name.clone(), kp.pubkey()));
        channels.insert(
            name.clone(),
            Channel {
                program_id: Some(kp.pubkey()),
                keypair: Some(rel),
                cluster: solex_types::Clusters::from(
                    opts.clusters
                        .get(name)
                        .map(String::as_str)
                        .unwrap_or("localnet"),
                ),
                upgrade_authority: None,
                includes: if i == 0 {
                    vec![]
                } else {
                    vec![opts.channels[i - 1].clone()]
                },
                description: None,
                vars: IndexMap::new(),
                overrides: IndexMap::new(),
            },
        );
    }
    // The template's `reset` instruction is a channel-managed Cargo feature
    // enabled on the second channel (and epoch-gated there) when one exists.
    let mut features = IndexMap::new();
    if let Some(second) = opts.channels.get(1) {
        features.insert(
            "reset".to_string(),
            Feature {
                description: Some("resets the counter; canary-only until proven".into()),
                channels: vec![second.clone()],
                epoch: IndexMap::from([(second.clone(), 1000u64)]),
            },
        );
    }
    let manifest = Manifest {
        name: Some(opts.name.clone()),
        program_path: None,
        build: Default::default(),
        channels: Channels {
            default_channel: Some(opts.channels[0].clone()),
            channels,
            features,
        },
    };
    manifest.validate()?;
    let manifest_file = match opts.format {
        Format::Toml => "Solex.toml",
        Format::Yaml => "Solex.yaml",
    };

    let module = opts.name.replace('-', "_");
    let first = &opts.channels[0];
    let second = opts.channels.get(1).cloned();

    write(
        &root,
        &mut files,
        manifest_file,
        &manifest_text(&manifest, opts.format)?,
    )?;
    write(
        &root,
        &mut files,
        "Cargo.toml",
        &cargo_toml(&opts.name, &module, &opts.solex_version),
    )?;
    write(&root, &mut files, "build.rs", &build_rs(manifest_file))?;
    write(
        &root,
        &mut files,
        "src/lib.rs",
        &lib_rs(&module, first, second.as_deref()),
    )?;
    write(&root, &mut files, ".gitignore", GITIGNORE)?;
    write(
        &root,
        &mut files,
        "README.md",
        &readme(&opts.name, &opts.channels),
    )?;

    Ok(Created {
        root,
        files,
        program_ids,
    })
}

fn write(root: &Path, files: &mut Vec<PathBuf>, rel: &str, contents: &str) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CoreError::io("create", parent, e))?;
    }
    std::fs::write(&path, contents).map_err(|e| CoreError::io("write", &path, e))?;
    files.push(PathBuf::from(rel));
    Ok(())
}

fn manifest_text(m: &Manifest, format: Format) -> Result<String> {
    Ok(format!(
        "# Solex release channels. Each channel is a separate deployment with its own\n\
         # program id; `includes` lists channels whose tags also ship here. Features are\n\
         # Cargo features enabled per channel; code gates on #[cfg(feature = ...)].\n\
         # Docs: https://github.com/lgalabru/solex\n{}",
        solex_types::to_string(m, format)?
    ))
}

fn cargo_toml(name: &str, module: &str, solex_version: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

# Standalone package even when nested inside another cargo workspace.
[workspace]

[lib]
crate-type = ["cdylib", "lib"]
name = "{module}"

[features]
default = []
cpi = ["no-entrypoint"]
no-entrypoint = []
idl-build = ["anchor-lang/idl-build"]
# Channel-managed: enabled per channel by Solex.toml [features.reset].
reset = []

[dependencies]
anchor-lang = "1"
solex-lang = "{solex_version}"

[profile.release]
overflow-checks = true
lto = "fat"
codegen-units = 1

# Anchor's entrypoint checks cfgs that only exist on the SBF target.
[lints.rust.unexpected_cfgs]
level = "warn"
check-cfg = [
    'cfg(target_os, values("solana"))',
    'cfg(feature, values("custom-heap", "custom-panic", "anchor-debug", "no-log-ix-name"))',
]
"#
    )
}

fn build_rs(manifest_file: &str) -> String {
    format!(
        r#"fn main() {{
    // Solex macros read the channel at expansion time; rebuild when it changes.
    println!("cargo:rerun-if-env-changed=SOLEX_CHANNEL");
    println!("cargo:rerun-if-env-changed=SOLEX_MANIFEST");
    println!("cargo:rerun-if-env-changed=SOLEX_CLUSTER");
    println!("cargo:rerun-if-changed={manifest_file}");
}}
"#
    )
}

fn lib_rs(module: &str, first: &str, second: Option<&str>) -> String {
    let canary_block = match second {
        Some(ch) => format!(
            r#"
    /// A channel-managed feature: Solex.toml enables `reset` on `{ch}` (and
    /// channels including it) and gives it an activation epoch there, so this
    /// handler is absent from other channels and inactive until epoch 1000.
    #[cfg(feature = "reset")]
    #[solex::activation("reset")]
    pub fn reset(ctx: Context<Update>) -> Result<()> {{
        ctx.accounts.counter.count = 0;
        Ok(())
    }}
"#
        ),
        None => String::new(),
    };
    format!(
        r#"use anchor_lang::prelude::*;

// Program id of the channel being built, from the solex manifest.
solex::declare_id!();

#[solex::program]
pub mod {module} {{
    use super::*;

    /// Ships on `{first}` and every channel that includes it.
    #[channel({first})]
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {{
        ctx.accounts.counter.count = 0;
        msg!("{module} ({{}}) initialized", solex::active_channel!());
        Ok(())
    }}

    #[channel({first})]
    pub fn increment(ctx: Context<Update>) -> Result<()> {{
        ctx.accounts.counter.count += 1;
        Ok(())
    }}
{canary_block}}}

#[account]
#[derive(InitSpace)]
pub struct Counter {{
    pub count: u64,
}}

#[derive(Accounts)]
pub struct Initialize<'info> {{
    #[account(init, payer = payer, space = 8 + Counter::INIT_SPACE)]
    pub counter: Account<'info, Counter>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}}

#[derive(Accounts)]
pub struct Update<'info> {{
    #[account(mut)]
    pub counter: Account<'info, Counter>,
}}
"#
    )
}

const GITIGNORE: &str =
    "/target\n# Channel keypairs carry deploy authority; keep them out of git.\n/keys/*.json\n";

fn readme(name: &str, channels: &[String]) -> String {
    let list = channels
        .iter()
        .map(|c| format!("- `{c}`"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# {name}\n\nA Solana program managed by [solex](https://github.com/lgalabru/solex).\n\n\
         Channels:\n{list}\n\n```sh\nsolex features            # what ships where\nsolex build --channel {first}\nsolex deploy --channel {first}\n```\n",
        first = channels[0]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_consistent_project() {
        let dir = tempfile::tempdir().unwrap();
        let opts = InitOptions {
            name: "demo-counter".into(),
            channels: vec!["stable".into(), "canary".into()],
            clusters: BTreeMap::from([("stable".into(), "mainnet".into())]),
            solex_version: "0.1".into(),
            format: Format::Toml,
        };
        let created = init(dir.path(), &opts).unwrap();
        let loaded = solex_types::load(&created.root.join("Solex.toml")).unwrap();
        let m = loaded.channels();
        let build_rs = std::fs::read_to_string(created.root.join("build.rs")).unwrap();
        assert!(build_rs.contains("rerun-if-changed=Solex.toml"));
        assert_eq!(m.default_channel.as_deref(), Some("stable"));
        assert_eq!(m.channels["canary"].includes, ["stable"]);
        assert_eq!(m.channels["stable"].cluster.single(), Some("mainnet"));
        assert_eq!(m.channels["canary"].cluster.single(), Some("localnet"));
        // keypair on disk matches the manifest program id
        for (ch, id) in &created.program_ids {
            let kp = crate::keys::pubkey_of(&created.root.join("keys").join(format!("{ch}.json")))
                .unwrap();
            assert_eq!(&kp, id);
            assert_eq!(m.program_id(ch).unwrap(), id);
        }
        let lib = std::fs::read_to_string(created.root.join("src/lib.rs")).unwrap();
        assert!(lib.contains("pub mod demo_counter"));
        assert!(lib.contains("#[cfg(feature = \"reset\")]"));
        assert!(lib.contains("#[solex::activation(\"reset\")]"));
        assert_eq!(m.features_for("canary").unwrap(), ["reset"]);
        assert!(m.features_for("stable").unwrap().is_empty());
        assert!(syn::parse_file(&lib).is_ok(), "generated lib.rs must parse");
    }

    #[test]
    fn init_refuses_existing_dir_and_bad_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = InitOptions {
            name: "x".into(),
            channels: vec!["stable".into()],
            clusters: BTreeMap::new(),
            solex_version: "0.1".into(),
            format: Format::Yaml,
        };
        std::fs::create_dir(dir.path().join("x")).unwrap();
        assert!(init(dir.path(), &opts).is_err());
        opts.name = "bad name".into();
        assert!(init(dir.path(), &opts).is_err());
        opts.name = "ok".into();
        opts.channels = vec!["Stable".into()];
        assert!(init(dir.path(), &opts).is_err());
    }
}
