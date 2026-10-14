use std::{collections::BTreeMap, path::PathBuf};

use clap::Args;
use solex_core::{CoreError, Result, scaffold};

use crate::style;

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
        };
        let created = scaffold::init(&self.path, &opts)?;

        println!();
        println!(
            "  {} Created {} in {}",
            style::ok(),
            style::bold(&self.name),
            style::dim(&created.root.display().to_string())
        );
        println!();
        println!("  {}", style::bold("Channels"));
        for (ch, id) in &created.program_ids {
            println!("    {} {}", style::pad_then(ch, 12, style::accent), id);
        }
        println!();
        println!("  {}", style::bold("Next"));
        println!("    cd {}", self.name);
        println!(
            "    solex features                 {}",
            style::dim("# what ships where")
        );
        println!(
            "    solex build --channel {}    {}",
            self.channels[0],
            style::dim("# cargo build-sbf for one channel")
        );
        println!();
        println!(
            "  {} keys/*.json hold deploy authority for each channel; they are gitignored.",
            style::warn()
        );
        println!();
        Ok(())
    }
}
