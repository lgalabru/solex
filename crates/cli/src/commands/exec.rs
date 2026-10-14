use std::{path::PathBuf, process::Command};

use clap::Args;
use solex_core::{CoreError, Result, build};

use crate::commands;

/// Run any command with the channel selected, from the program directory:
/// `solex exec -c nightly -- cargo +nightly build-bpf`.
#[derive(Args, Debug, Default)]
pub struct ExecCommand {
    /// Channel (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    /// Command and arguments (after `--`)
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

impl ExecCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channel = project.channels().resolve(self.channel.as_deref())?;
        let mut cmd = Command::new(&self.command[0]);
        cmd.args(&self.command[1..])
            .envs(build::channel_env(&project, &channel))
            .current_dir(project.program_dir());
        let status = cmd.status().map_err(|source| CoreError::Spawn {
            program: self.command[0].clone(),
            source,
        })?;
        if !status.success() {
            return Err(CoreError::CommandFailed {
                command: self.command.join(" "),
                code: status.code().unwrap_or(1),
                output: String::new(),
            });
        }
        Ok(())
    }
}
