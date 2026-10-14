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
            let mut cmd = Command::new("cargo");
            cmd.arg("test")
                .arg("--manifest-path")
                .arg(project.program_manifest())
                .args(&self.args)
                .envs(build::channel_env(&project, channel))
                .current_dir(project.root());
            build::run(&mut cmd, "cargo test", true)?;
        }
        Ok(())
    }
}
