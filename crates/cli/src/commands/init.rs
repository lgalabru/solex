use std::{collections::BTreeMap, path::PathBuf};

use clap::{Args, ValueEnum};
use solex_core::{CoreError, Result, scaffold, types::Format};
use solex_types;

use crate::{components, components::boxed, style};

#[derive(Args, Debug)]
pub struct InitCommand {
    /// Project name (also the crate name)
    pub name: String,

    /// Channels to create, first one is the default. Each later channel
    /// includes the previous one.
    #[arg(long, value_delimiter = ',', default_value = "stable,canary")]
    pub channels: Vec<String>,

    /// Cluster for a channel, as `channel=cluster` (repeatable).
    /// Example: --cluster stable=mainnet-beta --cluster canary=devnet
    #[arg(long = "cluster", value_name = "CHANNEL=CLUSTER")]
    pub clusters: Vec<String>,

    /// Directory to create the project in (default: current directory)
    #[arg(long, default_value = ".")]
    pub path: PathBuf,

    /// Manifest format
    #[arg(long, value_enum, default_value_t = ManifestFormat::Yaml)]
    pub format: ManifestFormat,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum ManifestFormat {
    Yaml,
    Toml,
}

impl From<ManifestFormat> for Format {
    fn from(f: ManifestFormat) -> Self {
        match f {
            ManifestFormat::Yaml => Format::Yaml,
            ManifestFormat::Toml => Format::Toml,
        }
    }
}

impl InitCommand {
    pub fn execute(&self) -> Result<()> {
        let mut clusters = BTreeMap::new();
        for entry in &self.clusters {
            let Some((ch, cluster)) = entry.split_once('=') else {
                return Err(CoreError::message(format!(
                    "invalid --cluster `{entry}`; expected CHANNEL=CLUSTER"
                )));
            };
            clusters.insert(ch.trim().to_string(), cluster.trim().to_string());
        }
        for ch in clusters.keys() {
            if !self.channels.contains(ch) {
                return Err(CoreError::message(format!(
                    "--cluster names channel `{ch}` which is not in --channels"
                )));
            }
        }
        let opts = scaffold::InitOptions {
            name: self.name.clone(),
            channels: self.channels.clone(),
            clusters,
            solex_version: env!("CARGO_PKG_VERSION").to_string(),
            format: self.format.into(),
        };
        let created = scaffold::init(&self.path, &opts)?;

        let m = solex_types::load(&created.root.join(match opts.format {
            Format::Toml => "Solex.toml",
            Format::Yaml => "Solex.yaml",
        }))?;
        let sections: Vec<Vec<boxed::Line>> = created
            .program_ids
            .iter()
            .map(|(ch, id)| {
                let cluster = m
                    .channels()
                    .channel(ch)
                    .map(|c| c.cluster.0.join(", "))
                    .unwrap_or_default();
                boxed::section(
                    &[
                        boxed::styled(style::bold(&components::channel(ch)), ch),
                        boxed::plain(cluster),
                    ],
                    boxed::plain(""),
                    &[
                        ("program", boxed::plain(id.clone())),
                        ("keypair", boxed::plain(format!("keys/{ch}.json"))),
                    ],
                )
            })
            .collect();
        components::print_notice(
            components::NoticeLevel::Success,
            &format!("Created {}", self.name),
            &format!(
                "{}\n{} files, one keypair per channel",
                created.root.display(),
                created.files.len()
            ),
        );
        println!("{}", boxed::frame(&sections));
        let first = &self.channels[0];
        let steps = [
            (format!("cd {}", self.name), String::new()),
            ("solex features".to_string(), "what ships where".to_string()),
            (
                format!("solex build --{first}"),
                "cargo build-sbf for one channel".to_string(),
            ),
        ];
        let w = steps
            .iter()
            .map(|(c, _)| c.chars().count())
            .max()
            .unwrap_or(0);
        let mut body: Vec<String> = steps
            .iter()
            .map(|(c, d)| format!("{c:<w$}  {d}").trim_end().to_string())
            .collect();
        body.push("keys/*.json hold deploy authority for each channel and are gitignored.".into());
        components::print_notice(components::NoticeLevel::Info, "Next", &body.join("\n"));
        Ok(())
    }
}
