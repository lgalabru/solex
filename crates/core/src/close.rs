//! Tear a channel down: `solana program close` reclaims the ProgramData rent
//! and leaves the program address permanently non-executable.

use std::{path::PathBuf, process::Command};

use crate::{CoreError, Project, Result, build, keys};

#[derive(Debug, Clone, Default)]
pub struct CloseOptions {
    /// Override the channel's cluster / RPC URL.
    pub url: Option<String>,
    /// Upgrade authority (defaults to the channel's, then the Solana CLI default).
    pub authority: Option<PathBuf>,
    /// Where the reclaimed lamports go (defaults to the Solana CLI default keypair).
    pub recipient: Option<String>,
    /// Close the authority's stale buffer accounts instead of the program.
    pub buffers: bool,
}

#[derive(Debug, Clone)]
pub struct ClosePlan {
    pub channel: String,
    pub program_id: String,
    pub cluster: Option<String>,
    pub authority: Option<PathBuf>,
    pub recipient: Option<String>,
    pub buffers: bool,
    /// Closing a program on mainnet is the one action solex refuses without
    /// an explicit extra flag from the caller.
    pub is_mainnet: bool,
}

impl ClosePlan {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new("solana");
        cmd.args(["program", "close"]);
        if self.buffers {
            cmd.arg("--buffers");
        } else {
            cmd.arg(&self.program_id).arg("--bypass-warning");
        }
        if let Some(url) = &self.cluster {
            cmd.args(["--url", url]);
        }
        if let Some(auth) = &self.authority {
            cmd.arg("--authority").arg(auth);
        }
        if let Some(r) = &self.recipient {
            cmd.args(["--recipient", r]);
        }
        cmd
    }
}

pub fn plan(project: &Project, channel: &str, opts: &CloseOptions) -> Result<ClosePlan> {
    let channels = project.channels();
    let ch = channels.channel(channel)?;
    let program_id = channels.program_id(channel)?.to_string();
    keys::validate_pubkey(&program_id).map_err(|reason| {
        CoreError::message(format!(
            "channel `{channel}` has an invalid program_id `{program_id}`: {reason}"
        ))
    })?;
    let cluster = opts.url.clone().or_else(|| ch.cluster.clone());
    let is_mainnet = cluster
        .as_deref()
        .is_some_and(|c| c.contains("mainnet") || c == "m");
    Ok(ClosePlan {
        channel: channel.to_string(),
        program_id,
        cluster,
        authority: opts.authority.clone().or_else(|| {
            ch.upgrade_authority
                .as_ref()
                .map(|p| project.resolve_path(p))
        }),
        recipient: opts.recipient.clone(),
        buffers: opts.buffers,
        is_mainnet,
    })
}

/// Execute the plan. Returns the Solana CLI output.
pub fn close(plan: &ClosePlan, verbose: bool) -> Result<String> {
    let mut cmd = plan.command();
    build::run(&mut cmd, "solana program close", verbose)
}
