//! `solana program deploy` for one channel, using that channel's program id,
//! cluster and upgrade authority from `Solex.yaml`.

use std::{path::PathBuf, process::Command};

use crate::{CoreError, Project, Result, build, keys};

#[derive(Debug, Clone, Default)]
pub struct DeployOptions {
    /// Pick one of the channel's clusters, or an RPC URL when the channel
    /// declares none. With several declared and no choice, every cluster is
    /// deployed to.
    pub url: Option<String>,
    /// Fee payer keypair (`solana program deploy --keypair`).
    pub payer: Option<PathBuf>,
    /// Override the channel's upgrade authority.
    pub upgrade_authority: Option<PathBuf>,
    pub verbose: bool,
}

#[derive(Debug, Clone)]
pub struct DeployPlan {
    pub channel: String,
    pub program_id: String,
    pub so_path: PathBuf,
    pub cluster: Option<String>,
    /// Keypair passed as `--program-id` on a first deploy; `None` means the
    /// program id string is passed instead (upgrade of an existing program).
    pub program_keypair: Option<PathBuf>,
    pub upgrade_authority: Option<PathBuf>,
    pub payer: Option<PathBuf>,
}

impl DeployPlan {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new("solana");
        cmd.args(["program", "deploy"]).arg(&self.so_path);
        cmd.arg("--program-id");
        match &self.program_keypair {
            Some(kp) => cmd.arg(kp),
            None => cmd.arg(&self.program_id),
        };
        if let Some(url) = &self.cluster {
            cmd.args(["--url", url]);
        }
        if let Some(auth) = &self.upgrade_authority {
            cmd.arg("--upgrade-authority").arg(auth);
        }
        if let Some(payer) = &self.payer {
            cmd.arg("--keypair").arg(payer);
        }
        cmd
    }
}

/// One plan per target cluster. A channel with `cluster = ["a", "b"]` yields
/// two plans unless `opts.url` picks one.
pub fn plans(project: &Project, channel: &str, opts: &DeployOptions) -> Result<Vec<DeployPlan>> {
    let ch = project.channels().channel(channel)?;
    let clusters = ch.cluster.select(opts.url.as_deref())?;
    if clusters.is_empty() {
        return Ok(vec![plan_for(project, channel, None, opts)?]);
    }
    clusters
        .into_iter()
        .map(|c| plan_for(project, channel, Some(c), opts))
        .collect()
}

/// Validate everything needed for a deploy without touching the network.
pub fn plan(project: &Project, channel: &str, opts: &DeployOptions) -> Result<DeployPlan> {
    let ch = project.channels().channel(channel)?;
    let cluster = match opts.url.clone() {
        Some(u) => Some(ch.cluster.select(Some(&u))?.remove(0)),
        None => ch.cluster.single().map(str::to_string),
    };
    plan_for(project, channel, cluster, opts)
}

fn plan_for(
    project: &Project,
    channel: &str,
    cluster: Option<String>,
    opts: &DeployOptions,
) -> Result<DeployPlan> {
    let channels = project.channels();
    let ch = channels.channel(channel)?;
    let program_id = channels.program_id(channel)?.to_string();
    keys::validate_pubkey(&program_id).map_err(|reason| {
        CoreError::message(format!(
            "channel `{channel}` has an invalid program_id `{program_id}`: {reason}"
        ))
    })?;

    let so_path = build::find_so(&project.channel_out_dir(channel))?;

    // Use the keypair when it exists and matches; otherwise deploy by id so
    // upgrades work on machines that never held the program keypair.
    let keypair_path = project.channel_keypair(channel)?;
    let program_keypair = if keypair_path.is_file() {
        let actual = keys::pubkey_of(&keypair_path)?;
        if actual != program_id {
            return Err(CoreError::message(format!(
                "keypair {} is for {actual}, but channel `{channel}` declares program_id \
                 {program_id}\n  Fix Solex.yaml or regenerate with `solex keys new --channel \
                 {channel} --force`.",
                keypair_path.display()
            )));
        }
        Some(keypair_path)
    } else {
        None
    };

    let upgrade_authority = opts.upgrade_authority.clone().or_else(|| {
        ch.upgrade_authority
            .as_ref()
            .map(|p| project.resolve_path(p))
    });

    Ok(DeployPlan {
        channel: channel.to_string(),
        program_id,
        so_path,
        cluster,
        program_keypair,
        upgrade_authority,
        payer: opts.payer.clone(),
    })
}

/// Execute the plan. Returns the program id reported by the Solana CLI.
pub fn deploy(plan: &DeployPlan, verbose: bool) -> Result<String> {
    let mut cmd = plan.command();
    let out = build::run(&mut cmd, "solana program deploy", verbose)?;
    let reported = out
        .lines()
        .find_map(|l| l.strip_prefix("Program Id:"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| plan.program_id.clone());
    if reported != plan.program_id {
        return Err(CoreError::message(format!(
            "solana CLI reported program id {reported} but channel `{}` expects {}",
            plan.channel, plan.program_id
        )));
    }
    Ok(reported)
}
