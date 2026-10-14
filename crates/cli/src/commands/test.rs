use std::{path::PathBuf, process::Command};

use clap::Args;
use solex_core::{Result, build};

use crate::{commands, style};

#[derive(Args, Debug, Default)]
pub struct TestCommand {
    /// Channel to test against (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Run the suite once per channel
    #[arg(long, conflicts_with = "channel")]
    pub all: bool,

    /// Cluster within the channel (needed when it has per-cluster overrides)
    #[arg(long)]
    pub cluster: Option<String>,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    /// Arguments forwarded to `cargo test` (after `--`)
    #[arg(last = true)]
    pub args: Vec<String>,
}

impl TestCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channels = commands::select_channels(&project, self.channel.as_deref(), self.all)?;
        for channel in &channels {
            println!("  {} testing {}", style::dim("▶"), style::accent(channel));
            let features = build::channel_features(&project, channel, &[])?;
            let cluster = project
                .channels()
                .channel(channel)?
                .resolve_cluster(channel, self.cluster.as_deref())?;
            let mut cmd = Command::new("cargo");
            cmd.arg("test")
                .arg("--manifest-path")
                .arg(project.program_manifest());
            if !features.is_empty() {
                cmd.args(["--features", &features.join(",")]);
            }
            cmd.args(&self.args)
                .envs(build::target_env(&project, channel, cluster.as_deref()))
                .current_dir(project.root());
            build::run(&mut cmd, "cargo test", true)?;
        }
        Ok(())
    }
}
