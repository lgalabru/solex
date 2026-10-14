use std::path::PathBuf;

use clap::Args;
use solex_core::{
    Result, build,
    deploy::{self, DeployOptions},
};

use crate::{commands, components, components::boxed, style};

#[derive(Args, Debug, Default)]
pub struct DeployCommand {
    /// Channel to deploy (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Build the channel first
    #[arg(long)]
    pub build: bool,

    /// One of the channel's clusters (or an RPC URL when it declares none).
    /// A channel with several clusters deploys to all of them unless given.
    #[arg(long, short, visible_alias = "cluster")]
    pub url: Option<String>,

    /// Fee payer keypair (default: Solana CLI default keypair)
    #[arg(long, short)]
    pub keypair: Option<PathBuf>,

    /// Override the channel's upgrade authority keypair
    #[arg(long)]
    pub upgrade_authority: Option<PathBuf>,

    /// Print the deploy command(s) without running them
    #[arg(long)]
    pub dry_run: bool,

    /// Stream Solana CLI output
    #[arg(long, short)]
    pub verbose: bool,

    /// Path to the manifest (Solex.toml, Solex.yaml, or a host manifest with a [solex] section)
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl DeployCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channel = project.channels().resolve(self.channel.as_deref())?;

        if self.build {
            let opts = build::BuildOptions {
                verbose: self.verbose,
                ..Default::default()
            };
            let sp = style::spinner(&format!("Building {channel}..."));
            let r = build::build(&project, &channel, &opts);
            sp.finish_and_clear();
            r?;
        }

        let opts = DeployOptions {
            url: self.url.clone(),
            payer: self.keypair.clone(),
            upgrade_authority: self.upgrade_authority.clone(),
            verbose: self.verbose,
        };

        for plan in deploy::plans(&project, &channel, &opts)? {
            let cluster = plan.cluster.as_deref();
            let target = cluster.unwrap_or("(solana CLI default)");
            if self.dry_run {
                println!("{}", build::display_command(&plan.command()));
                continue;
            }
            println!(
                "{}",
                boxed::frame(&[boxed::section(
                    &[
                        boxed::styled(
                            style::bold(&components::channel(&plan.channel)),
                            &plan.channel
                        ),
                        boxed::plain(target),
                    ],
                    boxed::plain(""),
                    &[
                        ("program", boxed::plain(components::short(&plan.program_id))),
                        (
                            "artifact",
                            boxed::plain(components::rel(project.root(), &plan.so_path)),
                        ),
                        (
                            "authority",
                            boxed::plain(
                                plan.upgrade_authority
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default(),
                            ),
                        ),
                    ],
                )])
            );
            let sp = style::spinner(&format!("Deploying to {target}..."));
            let r = deploy::deploy(&plan, self.verbose);
            sp.finish_and_clear();
            let id = r?;
            components::print_notice(
                components::NoticeLevel::Success,
                &format!("Deployed {} to {target}", plan.channel),
                &format!(
                    "{}\n{}",
                    components::explorer_address_link(&id, cluster),
                    id
                ),
            );
        }
        Ok(())
    }
}
