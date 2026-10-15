//! `solana program deploy` for one channel, using that channel's program id,
//! cluster and upgrade authority from `Solex.yaml`.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

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
    /// Extra arguments appended to `solana program deploy` (e.g.
    /// `--skip-feature-verify`, `--with-compute-unit-price`).
    pub extra_args: Vec<String>,
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
    pub extra_args: Vec<String>,
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
            cmd.args(["--url", solex_types::solana_url(url)]);
        }
        if let Some(auth) = &self.upgrade_authority {
            cmd.arg("--upgrade-authority").arg(auth);
        }
        if let Some(payer) = &self.payer {
            cmd.arg("--keypair").arg(payer);
        }
        cmd.args(&self.extra_args);
        cmd
    }
}

/// SBPF version an artifact was compiled for: the ELF `e_flags` field, which
/// `cargo build-sbf --arch vN` sets to N.
pub fn sbpf_version(so: &Path) -> Option<u32> {
    let mut f = std::fs::File::open(so).ok()?;
    let mut header = [0u8; 64];
    f.read_exact(&mut header).ok()?;
    if &header[..4] != b"\x7fELF" {
        return None;
    }
    Some(u32::from_le_bytes([
        header[48], header[49], header[50], header[51],
    ]))
}

fn solana_version() -> Option<String> {
    let out = Command::new("solana").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1).map(str::to_string)
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
    let cluster = ch.resolve_cluster(channel, opts.url.as_deref())?;
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
    let resolved = ch.resolved(channel, cluster.as_deref());
    let program_id = resolved
        .program_id
        .ok_or_else(|| CoreError::message(format!("channel `{channel}` has no program_id")))?
        .to_string();
    keys::validate_pubkey(&program_id).map_err(|reason| {
        CoreError::message(format!(
            "channel `{channel}` has an invalid program_id `{program_id}`: {reason}"
        ))
    })?;

    let so_path = build::find_so(&project.target_out_dir(channel, cluster.as_deref()))?;

    // Use the keypair when it exists and matches; otherwise deploy by id so
    // upgrades work on machines that never held the program keypair.
    let keypair_path = project.target_keypair(channel, cluster.as_deref())?;
    let program_keypair = if keypair_path.is_file() {
        let actual = keys::pubkey_of(&keypair_path)?;
        if actual != program_id {
            return Err(CoreError::message(format!(
                "keypair {} is for {actual}, but channel `{channel}` declares program_id \
                 {program_id}\n  Point the manifest's `keypair` at the right file or fix \
                 `program_id`.",
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
        extra_args: opts.extra_args.clone(),
    })
}

/// The Solana CLI verifies the ELF against the SBPF versions it knows before
/// uploading anything. A CLI older than the artifact's `--arch` rejects it
/// even when the cluster already runs that version, so say what to do.
fn sbpf_version_error(plan: &DeployPlan, output: &str) -> CoreError {
    let arch = sbpf_version(&plan.so_path)
        .map(|v| format!("SBPF v{v}"))
        .unwrap_or_else(|| "an SBPF version".into());
    let cli = solana_version()
        .map(|v| format!("solana CLI {v}"))
        .unwrap_or_else(|| "the installed solana CLI".into());
    let flag = plan
        .cluster
        .as_deref()
        .map(|c| format!("--cluster {c} "))
        .unwrap_or_default();
    CoreError::message(format!(
        "{} targets {arch}; {cli} is too old to verify it and refused before uploading\n\
         Nothing was sent. Upgrade the Solana CLI to the release the cluster runs\n\
         (`solana cluster-version` on the target, then `agave-install init <version>`),\n\
         or skip the local check and let the cluster's loader decide:\n  \
         solex deploy --{} {flag}-- --skip-feature-verify\n\
         {}",
        plan.so_path.display(),
        plan.channel,
        output.trim()
    ))
}

/// Execute the plan. Returns the program id reported by the Solana CLI.
pub fn deploy(plan: &DeployPlan, verbose: bool) -> Result<String> {
    let mut cmd = plan.command();
    let out = match build::run(&mut cmd, "solana program deploy", verbose) {
        Ok(out) => out,
        Err(CoreError::CommandFailed { output, .. }) if output.contains("sbpf_version") => {
            return Err(sbpf_version_error(plan, &output));
        }
        Err(e) => return Err(e),
    };
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
