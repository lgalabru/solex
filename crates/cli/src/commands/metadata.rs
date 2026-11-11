use std::{collections::HashSet, io::IsTerminal, path::PathBuf};

use clap::Args;
use dialoguer::{Confirm, theme::ColorfulTheme};
use solex_core::{
    CoreError, Project, Result, build,
    metadata::{self, MetadataOptions, MetadataPlan, OnChain},
};

use crate::{
    commands,
    components::{self, ChannelCard, NoticeLevel},
    style,
};

/// Upload a channel's program metadata (IDL, security.txt, ...) declared under
/// `[metadata.<seed>]`, through the Program Metadata program.
#[derive(Args, Debug, Default)]
pub struct MetadataCommand {
    /// Channel whose deployment gets the metadata (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// One of the channel's clusters; all of them when omitted
    #[arg(long, short, visible_alias = "cluster")]
    pub url: Option<String>,

    /// RPC endpoint instead of the cluster's public one
    #[arg(long)]
    pub rpc: Option<String>,

    /// Only this seed (repeatable); every declared seed by default
    #[arg(long = "seed", short = 's')]
    pub seeds: Vec<String>,

    /// Metadata authority keypair (default: the channel's upgrade authority, else the Solana CLI keypair)
    #[arg(long, short)]
    pub keypair: Option<PathBuf>,

    /// Fee and rent payer keypair, when different from the authority
    #[arg(long)]
    pub payer: Option<PathBuf>,

    /// Priority fee in micro-lamports per compute unit
    #[arg(long)]
    pub priority_fees: Option<u64>,

    /// Use the file as it is; do not run the seed's command
    #[arg(long)]
    pub no_generate: bool,

    /// Write even when the chain already holds the same content
    #[arg(long)]
    pub force: bool,

    /// Close the authority's leftover metadata buffers (interrupted writes) and reclaim their rent
    #[arg(long)]
    pub close_buffers: bool,

    /// Print the commands without running anything
    #[arg(long)]
    pub dry_run: bool,

    /// Write without asking; for CI and scripts
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Stream command output
    #[arg(long, short)]
    pub verbose: bool,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl MetadataCommand {
    pub fn options(&self) -> MetadataOptions {
        MetadataOptions {
            cluster: self.url.clone(),
            rpc: self.rpc.clone(),
            authority: self.keypair.clone(),
            payer: self.payer.clone(),
            seeds: self.seeds.clone(),
            priority_fees: self.priority_fees,
        }
    }

    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channel = project.channels().resolve(self.channel.as_deref())?;
        self.run(&project, &channel, self.options())
    }

    /// Shared with `solex deploy --metadata`, which pins the cluster it just
    /// deployed to.
    pub fn run(&self, project: &Project, channel: &str, opts: MetadataOptions) -> Result<()> {
        let plans = metadata::plans(project, channel, &opts)?;
        if self.close_buffers {
            return self.reclaim(&plans);
        }
        if self.dry_run {
            let mut seen = HashSet::new();
            for plan in &plans {
                if let Some(cmd) = plan.generate_command().filter(|_| !self.no_generate) {
                    let line = build::display_command(&cmd);
                    if seen.insert((line.clone(), plan.file.clone())) {
                        println!("{line}");
                    }
                }
                println!("{}", plan.describe());
            }
            return Ok(());
        }

        let mut generated = HashSet::new();
        for plan in &plans {
            if !self.no_generate {
                // Channels without per-cluster overrides produce one file for
                // every cluster: generate it once.
                let key = (
                    plan.generate.clone(),
                    plan.file.clone(),
                    plan.cluster
                        .clone()
                        .filter(|_| has_overrides(project, channel)),
                );
                if generated.insert(key) {
                    let sp = style::spinner(&format!("Generating {}...", plan.label()));
                    let r = metadata::generate(plan, self.verbose);
                    sp.finish_and_clear();
                    r?;
                }
            } else {
                metadata::generate(&without_command(plan), false)?;
            }
            self.write_one(project, plan)?;
        }
        Ok(())
    }

    /// One pass per RPC: the authority is the same for every seed.
    fn reclaim(&self, plans: &[MetadataPlan]) -> Result<()> {
        let mut seen = HashSet::new();
        for plan in plans {
            if !seen.insert(plan.rpc.clone()) {
                continue;
            }
            let rpc = plan.rpc_display();
            let target = plan.cluster.as_deref().unwrap_or(rpc.as_str());
            let sp = style::spinner(&format!("Listing buffers on {target}..."));
            let found = metadata::buffers(plan);
            sp.finish_and_clear();
            let found = found?;
            if found.is_empty() {
                components::print_notice(
                    NoticeLevel::Success,
                    &format!("No metadata buffers on {target}"),
                    "Nothing to close.",
                );
                continue;
            }
            let total: u64 = found.iter().map(|(_, l)| l).sum();
            let lines: Vec<String> = found
                .iter()
                .map(|(a, l)| format!("{a}  {:.6} SOL", *l as f64 / 1e9))
                .collect();
            components::print_notice_lines(
                NoticeLevel::Warning,
                &format!(
                    "{} buffer(s) on {target} hold {:.6} SOL",
                    found.len(),
                    total as f64 / 1e9
                ),
                &lines,
            );
            if self.dry_run {
                continue;
            }
            if !self.yes && std::io::stdin().is_terminal() {
                let ok = Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Close them?")
                    .default(true)
                    .interact_opt()
                    .map_err(|e| CoreError::message(format!("prompt failed: {e}")))?
                    .unwrap_or(false);
                if !ok {
                    continue;
                }
            }
            let addrs: Vec<String> = found.into_iter().map(|(a, _)| a).collect();
            let sp = style::spinner("Closing...");
            let r = metadata::close_buffers(plan, &addrs);
            sp.finish_and_clear();
            r?;
            components::print_notice(
                NoticeLevel::Success,
                &format!("Closed {} buffer(s) on {target}", addrs.len()),
                &format!("{:.6} SOL returned to the payer", total as f64 / 1e9),
            );
        }
        Ok(())
    }

    fn write_one(&self, project: &Project, plan: &MetadataPlan) -> Result<()> {
        let checked = metadata::check(plan)?;
        let sp = style::spinner(&format!("Reading {} on chain...", plan.label()));
        let status = metadata::on_chain(plan);
        sp.finish_and_clear();
        let status = status?;

        let state = match status {
            OnChain::Missing => "not on chain yet",
            OnChain::Same => "up to date",
            OnChain::Differs => "differs from chain",
        };
        let rpc = plan.rpc_display();
        let target = plan.cluster.as_deref().unwrap_or(rpc.as_str());
        println!(
            "{}",
            ChannelCard::new(&plan.channel, Some(target))
                .program(&plan.program_id)
                .text("seed", plan.seed.clone())
                .artifact(project.root(), &plan.file)
                .text(
                    "address",
                    if checked {
                        "matches the channel"
                    } else {
                        "not checked"
                    }
                )
                .text(
                    "authority",
                    metadata::authority_path(plan)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                )
                .text("status", state)
                .frame()
        );

        if status == OnChain::Same && !self.force {
            components::print_notice(
                NoticeLevel::Success,
                &format!("{} is up to date", plan.label()),
                "Nothing sent; --force writes anyway.",
            );
            return Ok(());
        }

        if !self.yes && std::io::stdin().is_terminal() {
            let ok = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!("Write {} to {target}?", plan.seed))
                .default(true)
                .interact_opt()
                .map_err(|e| CoreError::message(format!("prompt failed: {e}")))?
                .unwrap_or(false);
            if !ok {
                components::print_notice(NoticeLevel::Warning, "Nothing sent", "Skipped.");
                return Ok(());
            }
        }

        let sp = style::spinner(&format!("Writing {}...", plan.label()));
        let r = metadata::write(plan);
        sp.finish_and_clear();
        let report = r?;
        let address = solex_core::solana::b58(&report.metadata);
        components::print_notice_lines(
            NoticeLevel::Success,
            &format!(
                "{} {} for {} on {target} ({} transactions)",
                capitalize(report.action),
                plan.seed,
                plan.channel,
                report.signatures.len()
            ),
            &[format!(
                "{}  {}",
                components::explorer_link(&address, &address, plan.cluster.as_deref()),
                components::click_hint("Explorer")
            )],
        );
        Ok(())
    }
}

fn has_overrides(project: &Project, channel: &str) -> bool {
    project
        .channels()
        .channel(channel)
        .map(|c| c.has_overrides())
        .unwrap_or(false)
}

fn without_command(plan: &MetadataPlan) -> MetadataPlan {
    let mut p = plan.clone();
    p.generate = None;
    p
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}
