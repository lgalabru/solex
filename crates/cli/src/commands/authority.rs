use std::{io::IsTerminal, path::PathBuf};

use clap::Args;
use dialoguer::{Input, theme::ColorfulTheme};
use solex_core::{
    CoreError, Result,
    handover::{self, Handover, Status, Target},
    solana,
};

use crate::{
    commands,
    components::{self, ChannelCard, NoticeLevel},
    style,
};

/// Compare a deployment's upgrade authority with the manifest, and hand the
/// program over to the multisig the manifest declares.
#[derive(Args, Debug, Default)]
pub struct AuthorityCommand {
    /// Channel to look at (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Which of the channel's clusters (required when it declares several)
    #[arg(long, short, visible_alias = "cluster")]
    pub url: Option<String>,

    /// Keypair holding the authority today; it also pays the fee
    /// (default: Solana CLI default keypair)
    #[arg(long, short)]
    pub keypair: Option<PathBuf>,

    /// Hand the program over to the declared multisig. Without it the checks
    /// run and nothing is sent.
    #[arg(long)]
    pub transfer: bool,

    /// The vault address, as the Squads app or a teammate gave it. It must be
    /// the vault solex derives from the manifest; asked for when omitted.
    #[arg(long, requires = "transfer")]
    pub vault: Option<String>,

    /// Required in addition to --transfer when the cluster is mainnet
    #[arg(long)]
    pub allow_mainnet: bool,

    /// Path to the manifest (Solex.toml, Solex.yaml, or a host manifest with a [solex] section)
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl AuthorityCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channel = project.channels().resolve(self.channel.as_deref())?;
        let target = handover::target(
            &project,
            &channel,
            self.url.as_deref(),
            self.keypair.clone(),
        )?;
        let sp = style::spinner(&format!(
            "Checking the upgrade authority on {}...",
            target.cluster
        ));
        let status = handover::status(&target);
        sp.finish_and_clear();
        match status? {
            Status::Settled(signer) => {
                println!(
                    "{}",
                    ChannelCard::new(&target.channel, Some(&target.cluster))
                        .program(&target.program_id)
                        .text("authority", signer.describe())
                        .frame()
                );
                let body = match &target.declared {
                    Some(_) => "The cluster agrees with the manifest.",
                    None => {
                        "The manifest declares no `upgrade_authority` for this deployment. \
                         To hand it to a multisig, declare `upgrade_authority = \
                         \"squads:<multisig account>\"`."
                    }
                };
                components::print_notice(NoticeLevel::Success, "Nothing to hand over", body);
                Ok(())
            }
            Status::Pending(handover) => self.hand_over(&target, &handover),
        }
    }

    fn hand_over(&self, target: &Target, handover: &Handover) -> Result<()> {
        let multisig = &handover.multisig;
        println!(
            "{}",
            ChannelCard::new(&target.channel, Some(&target.cluster))
                .program(&handover.program_id)
                .text(
                    "authority",
                    format!("{} ({})", handover.from, handover.from_path.display())
                )
                .text("hand to", handover.vault.clone())
                .text(
                    "multisig",
                    format!(
                        "{}, {} to approve",
                        solana::b58(&multisig.address),
                        multisig.describe()
                    )
                )
                .frame()
        );
        if !handover.warnings.is_empty() {
            components::print_notice_lines(NoticeLevel::Warning, "Check", &handover.warnings);
        }
        let rerun = format!(
            "solex authority --channel {} --cluster {} --transfer",
            target.channel, target.cluster
        );
        if !self.transfer {
            components::print_notice(
                NoticeLevel::Warning,
                "Nothing sent",
                &format!(
                    "A handover is permanent: from then on only the multisig can upgrade the\n\
                     program or pass the authority on.\n\
                     Re-run with --transfer to execute:\n{rerun}"
                ),
            );
            return Ok(());
        }
        if handover.is_mainnet && !self.allow_mainnet {
            return Err(CoreError::message(format!(
                "channel `{}` targets mainnet; pass --allow-mainnet together with --transfer",
                target.channel
            )));
        }
        let vault = self.confirmed_vault(handover)?;
        let sp = style::spinner("Handing the upgrade authority over...");
        let r = handover::execute(handover, &vault);
        sp.finish_and_clear();
        let signature = r?;
        components::print_notice_lines(
            NoticeLevel::Success,
            &format!(
                "{} on {} is now controlled by the multisig",
                target.channel, target.cluster
            ),
            &[
                format!("authority    {}", handover.vault),
                format!("transaction  {signature}"),
                String::new(),
                "`solex deploy` now uploads each upgrade to a buffer and hands it to the vault \
                 for the multisig to approve."
                    .to_string(),
            ],
        );
        Ok(())
    }

    /// The vault address from somewhere other than the manifest. Typing it
    /// is the confirmation: a wrong multisig in the manifest derives another
    /// vault and stops here.
    fn confirmed_vault(&self, handover: &Handover) -> Result<String> {
        if let Some(vault) = &self.vault {
            return Ok(vault.clone());
        }
        if !std::io::stdin().is_terminal() {
            return Err(CoreError::message(
                "pass --vault <address>: the vault as the Squads app shows it, to confirm the \
                 handover",
            ));
        }
        Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Paste the vault address of multisig {} from the Squads app",
                components::short(&handover.reference.multisig)
            ))
            .interact_text()
            .map_err(|e| CoreError::message(format!("prompt failed: {e}")))
    }
}
