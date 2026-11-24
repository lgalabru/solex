//! Program metadata (IDL, security.txt, ...) per channel deployment, written
//! to the Program Metadata program by [`crate::program_metadata`].
//!
//! Each `[metadata.<seed>]` entry names a file and, optionally, the command
//! that produces it. The command runs with the channel's environment plus
//! `SOLEX_PROGRAM_ID`, so one recipe yields the right content for every
//! channel. Before writing, an IDL is checked against the channel's program
//! id and compared with what is already on chain: identical content is not
//! sent again.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use solex_types::MetadataConfig;

use crate::{
    CoreError, Project, Result, authority, build,
    keys::Keypair,
    program_metadata::{self, Format, WriteReport, WriteRequest},
    solana::{self, Rpc},
};

/// Program id of the channel deployment a metadata command runs for.
pub const PROGRAM_ID_ENV: &str = "SOLEX_PROGRAM_ID";

#[derive(Debug, Clone, Default)]
pub struct MetadataOptions {
    /// One of the channel's clusters; every cluster when `None`.
    pub cluster: Option<String>,
    /// RPC endpoint overriding the cluster's public one.
    pub rpc: Option<String>,
    /// Metadata authority. Defaults to the channel's upgrade authority, then
    /// to the Solana CLI keypair.
    pub authority: Option<PathBuf>,
    /// Fee and rent payer, when different from the authority.
    pub payer: Option<PathBuf>,
    /// Only these seeds; all declared seeds when empty.
    pub seeds: Vec<String>,
    /// Micro-lamports per compute unit.
    pub priority_fees: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct MetadataPlan {
    pub channel: String,
    pub cluster: Option<String>,
    pub rpc: String,
    pub program_id: String,
    pub seed: String,
    pub file: PathBuf,
    pub format: Option<String>,
    /// Producer command, already carrying its environment and directory.
    pub generate: Option<Vec<String>>,
    pub authority: Option<PathBuf>,
    pub payer: Option<PathBuf>,
    pub priority_fees: Option<u64>,
    env: Vec<(&'static str, String)>,
    root: PathBuf,
}

/// What the chain holds for a plan's seed, compared with the local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnChain {
    /// No canonical metadata account yet.
    Missing,
    /// Same content as the local file; nothing to send.
    Same,
    /// Different content; `write` updates it.
    Differs,
}

/// `https://host/path?api-key=...` → `https://host/path?…`.
pub fn redact_url(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?…"),
        None => url.to_string(),
    }
}

/// JSON-RPC endpoint for a manifest cluster name: `SOLEX_RPC_<CLUSTER>`
/// (e.g. `SOLEX_RPC_DEVNET`) when set, else the public endpoint; anything
/// that is not a moniker is taken as an RPC URL.
pub fn rpc_url(cluster: &str) -> String {
    let moniker = solex_types::solana_url(cluster);
    let var = format!(
        "SOLEX_RPC_{}",
        match moniker {
            "mainnet-beta" => "MAINNET".to_string(),
            "localhost" => "LOCALNET".to_string(),
            other => other.to_ascii_uppercase().replace('-', "_"),
        }
    );
    if !moniker.contains("://") {
        if let Ok(url) = std::env::var(&var) {
            if !url.trim().is_empty() {
                return url.trim().to_string();
            }
        }
    }
    match moniker {
        "mainnet-beta" => "https://api.mainnet-beta.solana.com".into(),
        "devnet" => "https://api.devnet.solana.com".into(),
        "testnet" => "https://api.testnet.solana.com".into(),
        "localhost" => "http://127.0.0.1:8899".into(),
        other => other.into(),
    }
}

impl MetadataPlan {
    /// The producer, run at the project root with the channel's environment.
    pub fn generate_command(&self) -> Option<Command> {
        let argv = self.generate.as_ref()?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .envs(self.env.iter().map(|(k, v)| (*k, v.as_str())))
            .current_dir(&self.root);
        Some(cmd)
    }

    /// Metadata account this plan writes.
    pub fn metadata_address(&self) -> Result<String> {
        let program = solana::pubkey(&self.program_id)?;
        Ok(solana::b58(&program_metadata::canonical_address(
            &program, &self.seed,
        )?))
    }

    /// The RPC endpoint without its query string, which often carries an API key.
    pub fn rpc_display(&self) -> String {
        redact_url(&self.rpc)
    }

    /// One line for `--dry-run`.
    pub fn describe(&self) -> String {
        format!(
            "write {} for {} ({}) from {} via {}",
            self.seed,
            self.program_id,
            self.metadata_address().unwrap_or_default(),
            self.file.display(),
            self.rpc_display()
        )
    }

    /// Label for output: `canary/devnet idl`.
    pub fn label(&self) -> String {
        match &self.cluster {
            Some(c) => format!("{}/{c} {}", self.channel, self.seed),
            None => format!("{} {}", self.channel, self.seed),
        }
    }
}

/// One plan per (cluster, seed). Fails when the manifest declares no
/// metadata, a requested seed is unknown, or a cluster cannot be resolved to
/// an RPC endpoint.
pub fn plans(
    project: &Project,
    channel: &str,
    opts: &MetadataOptions,
) -> Result<Vec<MetadataPlan>> {
    let declared = &project.manifest().metadata;
    if declared.is_empty() {
        return Err(CoreError::message(
            "no [metadata.<seed>] entries in the manifest\n  Add one, e.g.\n  \
             [metadata.idl]\n  file = \"target/idl/program.json\"",
        ));
    }
    for s in &opts.seeds {
        if !declared.contains_key(s) {
            return Err(CoreError::message(format!(
                "no [metadata.{s}] in the manifest (declared: {})",
                declared.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
    }
    let ch = project.channels().channel(channel)?;
    let clusters: Vec<Option<String>> = match &opts.cluster {
        Some(c) => vec![Some(ch.cluster.select(Some(c))?.remove(0))],
        None if ch.cluster.is_empty() => vec![None],
        None => ch.cluster.iter().map(|c| Some(c.to_string())).collect(),
    };
    let mut out = Vec::new();
    for cluster in clusters {
        let rpc = match (&opts.rpc, &cluster) {
            (Some(r), _) => r.clone(),
            (None, Some(c)) => rpc_url(c),
            (None, None) => {
                return Err(CoreError::message(format!(
                    "channel `{channel}` declares no cluster; pass --rpc <url>"
                )));
            }
        };
        let resolved = ch.resolved(channel, cluster.as_deref());
        let program_id = resolved
            .program_id
            .ok_or_else(|| CoreError::message(format!("channel `{channel}` has no program_id")))?
            .to_string();
        let authority = match (&opts.authority, resolved.upgrade_authority) {
            (Some(p), _) => Some(p.clone()),
            (None, Some(p)) => authority::declared_keypair(project, p)?,
            (None, None) => None,
        };
        for (seed, entry) in declared {
            if !opts.seeds.is_empty() && !opts.seeds.contains(seed) {
                continue;
            }
            out.push(plan_for(
                project,
                channel,
                cluster.clone(),
                &rpc,
                &program_id,
                seed,
                entry,
                authority.clone(),
                opts,
            ));
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn plan_for(
    project: &Project,
    channel: &str,
    cluster: Option<String>,
    rpc: &str,
    program_id: &str,
    seed: &str,
    entry: &MetadataConfig,
    authority: Option<PathBuf>,
    opts: &MetadataOptions,
) -> MetadataPlan {
    let out_dir = project.target_out_dir(channel, cluster.as_deref());
    let out_rel = out_dir
        .strip_prefix(project.root())
        .unwrap_or(&out_dir)
        .to_string_lossy()
        .into_owned();
    let name = project.name();
    let file = entry
        .file
        .replace("{channel}", channel)
        .replace("{cluster}", cluster.as_deref().unwrap_or(""))
        .replace("{name}", &name)
        .replace("{lib}", &name.replace('-', "_"))
        .replace("{out}", &out_rel);
    let format = entry.format.clone().or_else(|| {
        Path::new(&file)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|e| match e {
                "json" => Some("json".to_string()),
                "yaml" | "yml" => Some("yaml".to_string()),
                "toml" => Some("toml".to_string()),
                _ => None,
            })
    });
    let mut env = build::target_env(project, channel, cluster.as_deref());
    env.push((PROGRAM_ID_ENV, program_id.to_string()));
    MetadataPlan {
        channel: channel.to_string(),
        cluster,
        rpc: rpc.to_string(),
        program_id: program_id.to_string(),
        seed: seed.to_string(),
        file: project.resolve_path(Path::new(&file)),
        format,
        generate: (!entry.command.is_empty()).then(|| entry.command.clone()),
        authority,
        payer: opts.payer.clone(),
        priority_fees: opts.priority_fees,
        env,
        root: project.root().to_path_buf(),
    }
}

/// Run the plan's producer, if any, then make sure the file exists.
pub fn generate(plan: &MetadataPlan, verbose: bool) -> Result<()> {
    if let Some(mut cmd) = plan.generate_command() {
        let label = plan
            .generate
            .as_ref()
            .map(|a| a.join(" "))
            .unwrap_or_default();
        build::run(&mut cmd, &label, verbose)?;
    }
    if !plan.file.is_file() {
        return Err(CoreError::message(format!(
            "{} not found for [metadata.{}]{}",
            plan.file.display(),
            plan.seed,
            if plan.generate.is_some() {
                "; the command ran but did not write it"
            } else {
                ""
            }
        )));
    }
    Ok(())
}

/// The program address an IDL declares: Codama (`program.publicKey`), Anchor
/// 0.30+ (`address`) or legacy Anchor (`metadata.address`).
pub fn idl_address(idl: &serde_json::Value) -> Option<&str> {
    idl.pointer("/program/publicKey")
        .or_else(|| idl.get("address"))
        .or_else(|| idl.pointer("/metadata/address"))
        .and_then(|v| v.as_str())
}

/// For the `idl` seed, refuse a file that names another program: an IDL
/// generated for stable uploaded to canary would decode against the wrong
/// address. Returns whether an address was found and checked.
pub fn check(plan: &MetadataPlan) -> Result<bool> {
    if plan.seed != "idl" || plan.format.as_deref() != Some("json") {
        return Ok(false);
    }
    let text = std::fs::read_to_string(&plan.file)
        .map_err(|e| CoreError::io("read", plan.file.clone(), e))?;
    let idl: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        CoreError::message(format!("{} is not valid JSON: {e}", plan.file.display()))
    })?;
    match idl_address(&idl) {
        Some(addr) if addr != plan.program_id => Err(CoreError::message(format!(
            "{} declares program {addr}, but channel `{}` deploys {}\n  \
             Generate the IDL for this channel (the command sees {PROGRAM_ID_ENV}).",
            plan.file.display(),
            plan.channel,
            plan.program_id
        ))),
        Some(_) => Ok(true),
        None => Ok(false),
    }
}

fn same_content(local: &[u8], remote: &[u8], json: bool) -> bool {
    if json {
        if let (Ok(a), Ok(b)) = (
            serde_json::from_slice::<serde_json::Value>(local),
            serde_json::from_slice::<serde_json::Value>(remote),
        ) {
            return a == b;
        }
    }
    local.trim_ascii() == remote.trim_ascii()
}

/// Read the canonical metadata account and compare it with the local file.
pub fn on_chain(plan: &MetadataPlan) -> Result<OnChain> {
    let rpc = Rpc::new(&plan.rpc);
    let program = solana::pubkey(&plan.program_id)?;
    let Some(remote) = program_metadata::fetch(&rpc, &program, &plan.seed)? else {
        return Ok(OnChain::Missing);
    };
    let local =
        std::fs::read(&plan.file).map_err(|e| CoreError::io("read", plan.file.clone(), e))?;
    Ok(
        if same_content(&local, &remote, plan.format.as_deref() == Some("json")) {
            OnChain::Same
        } else {
            OnChain::Differs
        },
    )
}

/// `keypair_path` from the Solana CLI config, else its default location.
pub fn solana_cli_keypair() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let config = home.join(".config/solana/cli/config.yml");
    let from_config = std::fs::read_to_string(config).ok().and_then(|text| {
        text.lines().find_map(|l| {
            l.trim()
                .strip_prefix("keypair_path:")
                .map(|v| PathBuf::from(v.trim().trim_matches(['"', '\''])))
        })
    });
    Some(from_config.unwrap_or_else(|| home.join(".config/solana/id.json")))
}

/// The authority keypair a plan signs with: explicit, the channel's upgrade
/// authority, else the Solana CLI keypair.
pub fn authority_path(plan: &MetadataPlan) -> Result<PathBuf> {
    plan.authority
        .clone()
        .or_else(solana_cli_keypair)
        .ok_or_else(|| CoreError::message("no authority keypair; pass --keypair"))
}

/// Buffers held by the plan's authority on its cluster.
pub fn buffers(plan: &MetadataPlan) -> Result<Vec<(String, u64)>> {
    let authority = Keypair::read(&authority_path(plan)?)?;
    let rpc = Rpc::new(&plan.rpc);
    Ok(program_metadata::buffers(&rpc, &authority.pubkey_bytes())?
        .into_iter()
        .map(|(a, l)| (solana::b58(&a), l))
        .collect())
}

/// Close the given buffers, rent to the payer (or the authority).
pub fn close_buffers(plan: &MetadataPlan, addresses: &[String]) -> Result<Vec<String>> {
    let authority = Keypair::read(&authority_path(plan)?)?;
    let payer = match &plan.payer {
        Some(p) => Some(Keypair::read(p)?),
        None => None,
    };
    let list = addresses
        .iter()
        .map(|a| solana::pubkey(a))
        .collect::<Result<Vec<_>>>()?;
    program_metadata::close_buffers(
        &Rpc::new(&plan.rpc),
        &list,
        &authority,
        payer.as_ref().unwrap_or(&authority),
        plan.priority_fees,
    )
}

/// Create or update the metadata account.
pub fn write(plan: &MetadataPlan) -> Result<WriteReport> {
    let authority = Keypair::read(&authority_path(plan)?)?;
    let payer = match &plan.payer {
        Some(p) => Some(Keypair::read(p)?),
        None => None,
    };
    let content =
        std::fs::read(&plan.file).map_err(|e| CoreError::io("read", plan.file.clone(), e))?;
    let rpc = Rpc::new(&plan.rpc);
    program_metadata::write(
        &rpc,
        &WriteRequest {
            program: solana::pubkey(&plan.program_id)?,
            seed: &plan.seed,
            content: &content,
            format: Format::parse(plan.format.as_deref()),
            authority: &authority,
            payer: payer.as_ref().unwrap_or(&authority),
            priority_fee: plan.priority_fees,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(extra: &str) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Solex.toml");
        std::fs::write(
            &manifest,
            format!(
                r#"
default_channel = "stable"
[channels.stable]
program_id = "CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX"
cluster = "mainnet"
[channels.canary]
includes = ["stable"]
program_id = "CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ"
cluster = ["devnet", "mainnet"]
upgrade_authority = "keys/auth.json"
{extra}
"#
            ),
        )
        .unwrap();
        let p = Project::load(&manifest).unwrap();
        (dir, p)
    }

    const IDL: &str = r#"
[metadata.idl]
command = ["just", "channel-idl"]
file = "{out}/idl.json"
"#;

    #[test]
    fn plans_per_cluster_with_channel_env() {
        let (_d, p) = project(IDL);
        let plans = plans(&p, "canary", &MetadataOptions::default()).unwrap();
        assert_eq!(plans.len(), 2);
        let devnet = &plans[0];
        assert_eq!(devnet.rpc, "https://api.devnet.solana.com");
        assert_eq!(
            devnet.program_id,
            "CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ"
        );
        assert!(devnet.file.ends_with("target/solex/canary/idl.json"));
        assert_eq!(devnet.format.as_deref(), Some("json"));
        assert!(
            devnet
                .authority
                .as_ref()
                .unwrap()
                .ends_with("keys/auth.json")
        );
        let cmd = devnet.generate_command().unwrap();
        let envs: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.into(), v?.to_str()?.into())))
            .collect();
        assert!(envs.contains(&("SOLEX_CHANNEL".into(), "canary".into())));
        assert!(envs.contains(&(
            PROGRAM_ID_ENV.into(),
            "CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ".into()
        )));
        assert_eq!(
            devnet.metadata_address().unwrap(),
            "H4vafhd6PQm6XMbo3qJdBuk8njHgqvZNCu2TZDhH45uc"
        );
    }

    #[test]
    fn cluster_and_rpc_overrides() {
        let (_d, p) = project(IDL);
        let opts = MetadataOptions {
            cluster: Some("mainnet".into()),
            rpc: Some("https://rpc.example".into()),
            ..Default::default()
        };
        let plans = plans(&p, "canary", &opts).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].cluster.as_deref(), Some("mainnet"));
        assert_eq!(plans[0].rpc, "https://rpc.example");
        assert!(
            super::plans(
                &p,
                "canary",
                &MetadataOptions {
                    cluster: Some("testnet".into()),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn missing_or_unknown_seed_is_an_error() {
        let (_d, p) = project("");
        assert!(plans(&p, "stable", &MetadataOptions::default()).is_err());
        let (_d, p) = project(IDL);
        let opts = MetadataOptions {
            seeds: vec!["security".into()],
            ..Default::default()
        };
        assert!(plans(&p, "stable", &opts).is_err());
    }

    #[test]
    fn idl_address_mismatch_is_refused() {
        let (d, p) = project(IDL);
        let plan = plans(&p, "stable", &MetadataOptions::default())
            .unwrap()
            .remove(0);
        std::fs::create_dir_all(plan.file.parent().unwrap()).unwrap();
        std::fs::write(
            &plan.file,
            r#"{"kind":"rootNode","program":{"publicKey":"CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ"}}"#,
        )
        .unwrap();
        assert!(check(&plan).is_err());
        std::fs::write(
            &plan.file,
            r#"{"address":"CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX"}"#,
        )
        .unwrap();
        assert!(check(&plan).unwrap());
        std::fs::write(&plan.file, r#"{"name":"x"}"#).unwrap();
        assert!(!check(&plan).unwrap());
        drop(d);
    }

    #[test]
    fn api_keys_are_not_printed() {
        assert_eq!(
            redact_url("https://devnet.helius-rpc.com/?api-key=secret"),
            "https://devnet.helius-rpc.com/?…"
        );
        assert_eq!(
            redact_url("https://api.devnet.solana.com"),
            "https://api.devnet.solana.com"
        );
    }

    #[test]
    fn json_compares_by_value() {
        assert!(same_content(
            b"{\"a\": 1,\n \"b\":2}",
            b"{\"b\":2,\"a\":1}",
            true
        ));
        assert!(!same_content(b"{\"a\":1}", b"{\"a\":2}", true));
        assert!(same_content(b"text\n", b"text", false));
    }

    #[test]
    fn invalid_metadata_entries_fail_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Solex.toml");
        std::fs::write(
            &manifest,
            "[channels.stable]\nprogram_id = \"CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX\"\n\
             [metadata.this-seed-is-way-too-long]\nfile = \"a.json\"\n",
        )
        .unwrap();
        assert!(Project::load(&manifest).is_err());
    }
}
