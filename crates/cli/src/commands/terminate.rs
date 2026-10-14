use std::path::PathBuf;

use clap::Args;
use solex_core::{
    CoreError, Result,
    terminate::{self, TerminateOptions},
};

use crate::{commands, style};

/// Terminate a channel: close its program (or the authority's buffers) and
/// reclaim the rent. The address can never execute code again.
#[derive(Args, Debug, Default)]
pub struct TerminateCommand {
    /// Channel to close (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Close stale buffer accounts of the authority instead of the program
    #[arg(long)]
    pub buffers: bool,

    /// Recipient of the reclaimed lamports (default: Solana CLI default keypair)
    #[arg(long)]
    pub recipient: Option<String>,

    /// Override the channel's upgrade authority keypair
    #[arg(long)]
    pub authority: Option<PathBuf>,

    /// Which of the channel's clusters (required when it declares several)
    #[arg(long, short, visible_alias = "cluster")]
    pub url: Option<String>,

    /// Print the command without running it
    #[arg(long)]
    pub dry_run: bool,

    /// Actually terminate. Without it the plan is printed and nothing is sent.
    #[arg(long)]
    pub yes: bool,

    /// Required in addition to --yes when the channel's cluster is mainnet
    #[arg(long)]
    pub allow_mainnet: bool,

    /// Stream Solana CLI output
    #[arg(long, short)]
    pub verbose: bool,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl TerminateCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channel = project.channels().resolve(self.channel.as_deref())?;
        let opts = TerminateOptions {
            url: self.url.clone(),
            authority: self.authority.clone(),
            recipient: self.recipient.clone(),
            buffers: self.buffers,
        };
        let plan = terminate::plan(&project, &channel, &opts)?;

        println!();
        println!(
            "  {:<10} {}",
            style::dim("channel"),
            style::accent(&plan.channel)
        );
        println!(
            "  {:<10} {}",
            style::dim("target"),
            if plan.buffers {
                "buffer accounts of the authority".to_string()
            } else {
                format!("program {}", plan.program_id)
            }
        );
        println!(
            "  {:<10} {}",
            style::dim("cluster"),
            plan.cluster.as_deref().unwrap_or("(solana CLI default)")
        );
        if let Some(a) = &plan.authority {
            println!("  {:<10} {}", style::dim("authority"), a.display());
        }
        println!(
            "  {:<10} {}",
            style::dim("recipient"),
            plan.recipient
                .as_deref()
                .unwrap_or("(solana CLI default keypair)")
        );
        println!();
        if !plan.buffers {
            println!(
                "  {} Closing a program is permanent: the address can never execute code again, \
                 and every account that references it is orphaned.",
                style::warn()
            );
            println!();
        }

        if self.dry_run {
            println!("{}", solex_core::build::display_command(&plan.command()));
            return Ok(());
        }
        if !self.yes {
            println!(
                "  Nothing sent. Re-run with {} to execute.",
                style::bold("--yes")
            );
            println!(
                "  {}",
                style::dim(&solex_core::build::display_command(&plan.command()))
            );
            println!();
            return Ok(());
        }
        if plan.is_mainnet && !self.allow_mainnet {
            return Err(CoreError::message(format!(
                "channel `{}` targets mainnet; pass --allow-mainnet together with --yes to terminate it",
                plan.channel
            )));
        }

        let sp = style::spinner("Terminating...");
        let r = terminate::terminate(&plan, self.verbose);
        sp.finish_and_clear();
        let out = r?;
        let reclaimed = out
            .lines()
            .find(|l| l.contains("Closed") || l.contains("SOL") || l.contains("lamports"))
            .map(str::trim)
            .unwrap_or("done");
        println!(
            "  {} Terminated {} on {}: {}",
            style::ok(),
            style::accent(&plan.channel),
            plan.cluster.as_deref().unwrap_or("default cluster"),
            reclaimed
        );
        println!();
        Ok(())
    }
}
