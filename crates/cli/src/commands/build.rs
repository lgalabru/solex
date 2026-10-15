use std::path::PathBuf;

use clap::Args;
use solex_core::{
    Result,
    build::{self, BuildOptions},
};

use crate::{
    commands, components,
    components::{ChannelCard, boxed},
    style,
};

#[derive(Args, Debug, Default)]
pub struct BuildCommand {
    /// Channel to build (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Build every channel
    #[arg(long, conflicts_with = "channel")]
    pub all: bool,

    /// Cluster within the channel; a channel with per-cluster overrides
    /// builds every cluster unless one is given
    #[arg(long)]
    pub cluster: Option<String>,

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
            cluster: None,
        };

        let untracked = solex_core::scan::untracked_env(&project.program_dir());
        if !untracked.is_empty() && !self.dry_run {
            let body = untracked
                .iter()
                .map(|v| format!("println!(\"cargo:rerun-if-env-changed={v}\");"))
                .collect::<Vec<_>>()
                .join("\n");
            components::print_notice(
                components::NoticeLevel::Warning,
                &format!(
                    "{} does not track {}; cargo may reuse another channel's compilation. Add:",
                    project.program_dir().join("build.rs").display(),
                    untracked.join(", ")
                ),
                &body,
            );
        }
        let mut sections: Vec<Vec<boxed::Line>> = Vec::new();
        for channel in &channels {
            // One build per cluster when overrides make the artifacts differ.
            let ch = project.channels().channel(channel)?;
            let targets: Vec<Option<String>> = match &self.cluster {
                Some(c) => vec![Some(c.clone())],
                None if ch.has_overrides() && ch.cluster.single().is_none() => {
                    ch.cluster.iter().map(|c| Some(c.to_string())).collect()
                }
                None => vec![None],
            };
            for cluster in targets {
                let opts = BuildOptions {
                    cluster,
                    ..opts.clone()
                };
                let cmd = build::build_command(&project, channel, &opts);
                if self.dry_run {
                    println!("{}", build::display_command(&cmd));
                    continue;
                }
                if self.verbose {
                    eprintln!("{}", style::dim(&build::display_command(&cmd)));
                }
                let label = match &opts.cluster {
                    Some(c) if ch.has_overrides() => format!("{channel}/{c}"),
                    _ => channel.clone(),
                };
                let sp = style::spinner(&format!("Building {label}..."));
                let result = build::build(&project, channel, &opts);
                sp.finish_and_clear();
                let out = result?;
                let features = build::channel_features(&project, channel, &opts.features)?;

                let size = format!(
                    "{}  {}",
                    style::human_size(out.size),
                    style::human_duration(out.elapsed)
                );
                sections.push(
                    ChannelCard::new(channel, out.cluster.as_deref())
                        .right(&size)
                        .program(&out.program_id)
                        .features(&features)
                        .artifact(project.root(), &out.so_path)
                        .section(),
                );
            }
        }
        if !sections.is_empty() {
            println!("{}", boxed::frame(&sections));
        }
        Ok(())
    }
}
