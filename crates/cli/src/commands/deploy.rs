use std::{io::IsTerminal, path::PathBuf};

use clap::Args;
use dialoguer::{MultiSelect, theme::ColorfulTheme};
use solex_core::{
    CoreError, Result,
    authority::{Kind, Signer},
    build,
    deploy::{self, DeployOptions},
    handoff,
};

use crate::{
    commands, commands::metadata::MetadataCommand, components, components::ChannelCard, style,
};

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

    /// After each deploy, upload the [metadata.<seed>] entries (IDL, ...) to the same cluster
    #[arg(long)]
    pub metadata: bool,

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

/// The command that hands this plan's program over to its declared multisig.
fn authority_command(plan: &deploy::DeployPlan) -> String {
    let cluster = plan
        .cluster
        .as_deref()
        .map(|c| format!(" --cluster {c}"))
        .unwrap_or_default();
    format!(
        "solex authority --channel {}{cluster} --transfer",
        plan.channel
    )
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
                match &plan.authority_address {
                    // The Solana CLI cannot sign for it, so there is no
                    // command to print.
                    Some(a) => println!(
                        "# {} on {target}: upgrade authority {a} has no keypair here",
                        plan.channel
                    ),
                    None => println!("{}", build::display_command(&plan.command())),
                }
                if self.metadata {
                    self.upload_metadata(&project, &plan)?;
                }
                continue;
            }
            let sp = style::spinner(&format!("Checking the upgrade authority on {target}..."));
            let signer = deploy::signer(&plan);
            sp.finish_and_clear();
            let signer = signer?;
            let features = build::channel_features(&project, &plan.channel, &[])?;
            println!(
                "{}",
                ChannelCard::new(&plan.channel, Some(target))
                    .program(&plan.program_id)
                    .features(&features)
                    .artifact(project.root(), &plan.so_path)
                    .text(
                        "authority",
                        match &signer {
                            Some(s) => s.describe(),
                            None => plan
                                .upgrade_authority
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                        },
                    )
                    .frame()
            );
            match &signer {
                Some(Signer::Remote {
                    address,
                    kind: Kind::Squads(multisig),
                    deployed: true,
                }) => {
                    self.hand_off_to_vault(&project, &plan, address, multisig, target)?;
                    continue;
                }
                Some(Signer::Remote {
                    address,
                    kind,
                    deployed,
                }) => {
                    self.hand_off(&project, &plan, address, kind, *deployed, target)?;
                    continue;
                }
                Some(Signer::Outgoing {
                    address,
                    to,
                    deployed: true,
                    ..
                }) => {
                    return Err(CoreError::message(format!(
                        "the manifest declares upgrade authority {to}, but {} on {target} is \
                         still controlled by {address}\n  Nothing was sent. Hand it over first: \
                         {}",
                        plan.program_id,
                        authority_command(&plan)
                    )));
                }
                // A first deploy under the local key, handed over afterwards.
                Some(Signer::Outgoing {
                    deployed: false, ..
                })
                | Some(Signer::Local { .. })
                | None => {}
            }
            let sp = style::spinner(&format!("Deploying to {target}..."));
            let r = deploy::deploy(&plan, self.verbose);
            sp.finish_and_clear();
            let id = r?;
            components::print_notice_lines(
                components::NoticeLevel::Success,
                &format!("Deployed {} to {target}", plan.channel),
                &[format!(
                    "{}  {}",
                    components::explorer_link(&id, &id, cluster),
                    components::click_hint("Explorer")
                )],
            );
            if self.metadata {
                self.upload_metadata(&project, &plan)?;
            }
            if let Some(Signer::Outgoing { address, to, .. }) = &signer {
                components::print_notice_lines(
                    components::NoticeLevel::Warning,
                    "The upgrade authority is not handed over yet",
                    &[
                        format!("on chain  {address}"),
                        format!("declared  {to}"),
                        String::new(),
                        format!("Hand it over: {}", authority_command(&plan)),
                    ],
                );
            }
        }
        Ok(())
    }

    /// The authority is a multisig vault: upload the buffer, hand it to the
    /// vault, and say what the multisig has to approve.
    fn hand_off_to_vault(
        &self,
        project: &solex_core::Project,
        plan: &deploy::DeployPlan,
        vault: &str,
        multisig: &solex_core::squads::Multisig,
        target: &str,
    ) -> Result<()> {
        let out_dir = project.target_out_dir(&plan.channel, plan.cluster.as_deref());
        let sp = style::spinner(&format!("Uploading the program buffer to {target}..."));
        let r = handoff::prepare_for_vault(plan, vault, &out_dir, self.verbose);
        sp.finish_and_clear();
        let h = r?;
        components::print_notice_lines(
            components::NoticeLevel::Warning,
            &format!(
                "{} on {target} is waiting for the multisig ({})",
                plan.channel,
                multisig.describe()
            ),
            &[
                format!("program  {}", plan.program_id),
                format!("buffer   {}", h.buffer),
                format!("spill    {}", h.payer),
                format!("vault    {}", h.vault),
                String::new(),
                "The program is not upgraded until the multisig approves and executes an \
                 upgrade from this buffer. Propose it in Squads as a program upgrade with the \
                 buffer and spill addresses above; the spill address gets the buffer's rent \
                 back."
                    .to_string(),
                "The buffer now belongs to the vault: only the multisig can use or close it."
                    .to_string(),
            ],
        );
        Ok(())
    }

    /// The authority's key is elsewhere: upload the buffer and leave the one
    /// transaction it has to sign, as a Vow review file when possible.
    fn hand_off(
        &self,
        project: &solex_core::Project,
        plan: &deploy::DeployPlan,
        address: &str,
        kind: &Kind,
        deployed: bool,
        target: &str,
    ) -> Result<()> {
        let stop = |what: String, next: &str| {
            Err(CoreError::message(format!(
                "{what}\n  Nothing was sent. {next}"
            )))
        };
        if !deployed {
            return stop(
                format!(
                    "{} is not deployed on {target} yet, and the keypair of its declared \
                     upgrade authority {address} is not on this machine",
                    plan.program_id
                ),
                "A first deploy to an authority held elsewhere is not built yet; deploy with a \
                 keypair, then hand the authority over.",
            );
        }
        if let Kind::Derived { owner } = kind {
            return stop(
                format!(
                    "the upgrade authority of {} on {target} is {address}, a program-derived \
                     address{}",
                    plan.program_id,
                    owner
                        .as_ref()
                        .map(|o| format!(" owned by {o}"))
                        .unwrap_or_default()
                ),
                "Only its program can sign for it. If it is the vault of a Squads multisig, \
                 declare `upgrade_authority = \"squads:<multisig account>\"` and solex hands \
                 each upgrade to the vault.",
            );
        }
        let out_dir = project.target_out_dir(&plan.channel, plan.cluster.as_deref());
        let sp = style::spinner(&format!("Uploading the program buffer to {target}..."));
        let r = handoff::prepare(plan, address, &out_dir, self.verbose);
        sp.finish_and_clear();
        let h = r?;
        let shown = |p: &std::path::Path| {
            p.strip_prefix(project.root())
                .unwrap_or(p)
                .display()
                .to_string()
        };
        let mut lines = vec![
            format!("buffer       {}", h.buffer),
            format!("signer       {address}"),
            format!("transaction  {}", shown(&h.transaction)),
        ];
        match (&h.review, &h.review_skipped) {
            (Some(path), _) => lines.push(format!("review file  {}", shown(path))),
            (None, Some(why)) => lines.push(format!("review file  none: {why}")),
            (None, None) => {}
        }
        lines.push(String::new());
        lines.push(
            "The program is not upgraded until the authority signs this transaction and it is \
             sent."
                .to_string(),
        );
        lines.push(
            "It uses a recent blockhash and expires about a minute from now, so for now it is \
             for review rather than for signing on another device."
                .to_string(),
        );
        lines.push(format!(
            "To give up and reclaim the buffer's rent: solana program close {} --url {}",
            h.buffer,
            solex_core::types::solana_url(target)
        ));
        components::print_notice_lines(
            components::NoticeLevel::Warning,
            &format!("{} on {target} is waiting for a signature", plan.channel),
            &lines,
        );
        Ok(())
    }

    /// The cluster was already chosen for the deploy, so the upload does not
    /// ask again.
    fn upload_metadata(
        &self,
        project: &solex_core::Project,
        plan: &deploy::DeployPlan,
    ) -> Result<()> {
        let cmd = MetadataCommand {
            dry_run: self.dry_run,
            yes: true,
            verbose: self.verbose,
            keypair: self.upgrade_authority.clone(),
            payer: self.keypair.clone(),
            ..Default::default()
        };
        let mut opts = cmd.options();
        opts.cluster = plan.cluster.clone();
        cmd.run(project, &plan.channel, opts)
    }
}
