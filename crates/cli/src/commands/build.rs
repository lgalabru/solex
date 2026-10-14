use std::path::PathBuf;

use clap::Args;
use solex_core::{
    Result,
    build::{self, BuildOptions},
};

use crate::{commands, style};

#[derive(Args, Debug, Default)]
pub struct BuildCommand {
    /// Channel to build (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Build every channel
    #[arg(long, conflicts_with = "channel")]
    pub all: bool,

    /// Cargo features to enable (comma-separated or repeated)
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,

    /// Keep debug symbols
    #[arg(long)]
    pub debug: bool,

    /// Pin the platform-tools version passed to cargo build-sbf
    #[arg(long, value_name = "VERSION")]
    pub tools_version: Option<String>,

    /// Stream compiler output
    #[arg(long, short)]
    pub verbose: bool,

    /// Print the build command(s) without running them
    #[arg(long)]
    pub dry_run: bool,

    /// Path to the manifest (default: search upward from the current directory)
    #[arg(long, global = true)]
    pub manifest: Option<PathBuf>,

    /// Extra arguments forwarded to `cargo build-sbf` (after `--`)
    #[arg(last = true)]
    pub sbf_args: Vec<String>,
}

impl BuildCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channels = commands::select_channels(&project, self.channel.as_deref(), self.all)?;
        commands::ensure_nonempty_channels(&channels)?;
        let opts = BuildOptions {
            features: self.features.clone(),
            verbose: self.verbose,
            debug: self.debug,
            tools_version: self.tools_version.clone(),
            extra_args: self.sbf_args.clone(),
        };

        for channel in &channels {
            let cmd = build::build_command(&project, channel, &opts);
            if self.dry_run {
                println!("{}", build::display_command(&cmd));
                continue;
            }
            if self.verbose {
                eprintln!("  {}", style::dim(&build::display_command(&cmd)));
            }
            let sp = style::spinner(&format!("Building {}...", style::accent(channel)));
            let result = build::build(&project, channel, &opts);
            sp.finish_and_clear();
            let out = result?;
            println!(
                "  {} {} {} {} {}",
                style::ok(),
                style::pad_then(&out.channel, 10, style::accent),
                style::dim(&out.program_id),
                style::dim(&style::human_size(out.size)),
                style::dim(&style::human_duration(out.elapsed)),
            );
            println!("    {}", style::dim(&out.so_path.display().to_string()));
        }
        Ok(())
    }
}
