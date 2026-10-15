use std::{io::IsTerminal, path::PathBuf};

use clap::Args;
use dialoguer::{MultiSelect, theme::ColorfulTheme};
use solex_core::{
    CoreError, Result, build,
    deploy::{self, DeployOptions},
};

use crate::{commands, components, components::ChannelCard, style};

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

    /// Deploy without asking which cluster(s); for CI and scripts
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Stream Solana CLI output
    #[arg(long, short)]
    pub verbose: bool,

    /// Path to the manifest (Solex.toml, Solex.yaml, or a host manifest with a [solex] section)
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    /// Extra arguments forwarded to `solana program deploy` (after `--`),
    /// e.g. `-- --skip-feature-verify` or `-- --with-compute-unit-price 1000`
    #[arg(last = true)]
    pub deploy_args: Vec<String>,
}

/// Ask which of the channel's clusters to deploy to: a multi-select with
/// every cluster checked, also for a single one so the gesture is always the
/// same. Escape or Ctrl-C answers "none".
fn pick_targets(plans: Vec<deploy::DeployPlan>) -> Result<Vec<deploy::DeployPlan>> {
    let items: Vec<String> = plans
        .iter()
        .map(|p| {
            format!(
                "{}  {}",
                p.cluster.as_deref().unwrap_or("solana CLI default cluster"),
                components::short(&p.program_id)
            )
        })
        .collect();
    let defaults = vec![true; items.len()];
    let picked = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Deploy {} to (space toggles, enter confirms)",
            plans[0].channel
        ))
        .items(&items)
        .defaults(&defaults)
        .interact_opt()
        .map_err(|e| CoreError::message(format!("prompt failed: {e}")))?
        .unwrap_or_default();
    Ok(plans
        .into_iter()
        .enumerate()
        .filter(|(i, _)| picked.contains(i))
        .map(|(_, p)| p)
        .collect())
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
            extra_args: self.deploy_args.clone(),
        };

        let mut plans = deploy::plans(&project, &channel, &opts)?;
        if !self.dry_run && !self.yes && std::io::stdin().is_terminal() {
            plans = pick_targets(plans)?;
            if plans.is_empty() {
                components::print_notice(
                    components::NoticeLevel::Warning,
                    "Nothing sent",
                    "No cluster selected.",
                );
                return Ok(());
            }
        }

        for plan in plans {
            let cluster = plan.cluster.as_deref();
            let target = cluster.unwrap_or("(solana CLI default)");
            if self.dry_run {
                println!("{}", build::display_command(&plan.command()));
                continue;
            }
            let features = build::channel_features(&project, &plan.channel, &[])?;
            println!(
                "{}",
                ChannelCard::new(&plan.channel, Some(target))
                    .program(&plan.program_id)
                    .features(&features)
                    .artifact(project.root(), &plan.so_path)
                    .text(
                        "authority",
                        plan.upgrade_authority
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                    )
                    .frame()
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
