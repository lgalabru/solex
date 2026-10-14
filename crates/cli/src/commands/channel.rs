use std::path::PathBuf;

use clap::Args;
use solex_core::Result;

use crate::commands;

#[derive(Args, Debug, Default)]
pub struct ChannelCommand {
    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl ChannelCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        println!("{}", project.channels().resolve(None)?);
        Ok(())
    }
}
