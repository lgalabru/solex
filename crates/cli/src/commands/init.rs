use std::{collections::BTreeMap, path::PathBuf};

use clap::{Args, ValueEnum};
use solex_core::{CoreError, Result, scaffold, types::Format};

use crate::{components, style};

#[derive(Args, Debug)]
pub struct InitCommand {
    /// Project name (also the crate name)
    pub name: String,

    /// Channels to create, first one is the default. Each later channel
    /// includes the previous one.
    #[arg(long, value_delimiter = ',', default_value = "stable,nightly")]
    pub channels: Vec<String>,

    /// Cluster for a channel, as `channel=cluster` (repeatable).
    /// Example: --cluster stable=mainnet-beta --cluster nightly=devnet
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

        println!(
            "{}",
            components::ok(&format!(
                "Created {} {}",
                style::bold(&self.name),
                style::dim(&created.root.display().to_string())
            ))
        );
        let rows: Vec<(&str, String)> = created
            .program_ids
            .iter()
            .map(|(ch, id)| (ch.as_str(), format!("{}  {}", components::channel(ch), id)))
            .collect();
        println!("{}", components::fields(&rows));
        components::print_notice(
            components::NoticeLevel::Info,
            "Next",
            &format!(
                "cd {}\nsolex features            what ships where\nsolex build --{}   \
                 cargo build-sbf for one channel\nkeys/*.json hold deploy authority for each \
                 channel and are gitignored.",
                self.name, self.channels[0]
            ),
        );
        Ok(())
    }
}
