//! An upgrade whose authority has no keypair on this machine: upload the
//! program to a buffer with the fee payer, then write the one transaction
//! the authority has to sign, as a Vow review file when `vow` is installed.
//!
//! The buffer stays under the fee payer's authority until that transaction
//! runs, so it can be closed and its rent reclaimed if the signature never
//! comes.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};

use crate::{
    CoreError, Result, build,
    deploy::DeployPlan,
    keys::{self, Keypair},
    metadata, program_metadata,
    solana::{self, AccountMeta, Instruction, Pubkey, Rpc},
};

const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
/// ProgramData header: u32 tag, u64 slot, Option<Pubkey> authority.
const PROGRAM_DATA_HEADER: usize = 45;

/// What was prepared for the authority to sign.
#[derive(Debug, Clone)]
pub struct Handoff {
    pub buffer: String,
    pub payer: String,
    /// The serialized transaction, base64, in a file.
    pub transaction: PathBuf,
    /// The Vow review file, when one could be made.
    pub review: Option<PathBuf>,
    /// Why there is no review file.
    pub review_skipped: Option<String>,
}

fn loader_ix(tag: u32, accounts: Vec<AccountMeta>) -> Result<Instruction> {
    Ok(Instruction {
        program_id: solana::pubkey(solana::LOADER_V3)?,
        accounts,
        data: tag.to_le_bytes().to_vec(),
    })
}

/// Hand the buffer to the upgrade authority and upgrade from it, atomically.
/// Signed by `payer` (fees, current buffer authority) and `authority`.
pub fn upgrade_instructions(
    program: &Pubkey,
    buffer: &Pubkey,
    payer: &Pubkey,
    authority: &Pubkey,
) -> Result<Vec<Instruction>> {
    let set_buffer_authority = loader_ix(
        4,
        vec![
            AccountMeta::writable(*buffer, false),
            AccountMeta::readonly(*payer, true),
            AccountMeta::readonly(*authority, false),
        ],
    )?;
    let upgrade = loader_ix(
        3,
        vec![
            AccountMeta::writable(program_metadata::program_data_address(program)?, false),
            AccountMeta::writable(*program, false),
            AccountMeta::writable(*buffer, false),
            // Spill: the buffer's rent goes back to whoever paid for it.
            AccountMeta::writable(*payer, false),
            AccountMeta::readonly(solana::pubkey(solana::RENT_SYSVAR)?, false),
            AccountMeta::readonly(solana::pubkey(CLOCK_SYSVAR)?, false),
            AccountMeta::readonly(*authority, true),
        ],
    )?;
    Ok(vec![set_buffer_authority, upgrade])
}

/// Where the payer keypair for the buffer comes from: `--keypair`, else the
/// Solana CLI's.
fn payer_path(plan: &DeployPlan) -> Result<PathBuf> {
    plan.payer
        .clone()
        .or_else(metadata::solana_cli_keypair)
        .filter(|p| p.is_file())
        .ok_or_else(|| CoreError::message("no fee payer keypair; pass --keypair"))
}

/// The Solana CLI refuses an upgrade that does not fit; say so before paying
/// for a buffer.
fn check_fits(rpc: &Rpc, plan: &DeployPlan, program: &Pubkey) -> Result<()> {
    let len = std::fs::metadata(&plan.so_path)
        .map(|m| m.len() as usize)
        .map_err(|source| CoreError::Io {
            action: "read",
            path: plan.so_path.clone(),
            source,
        })?;
    let capacity = rpc
        .account(&program_metadata::program_data_address(program)?)?
        .map(|a| a.data.len().saturating_sub(PROGRAM_DATA_HEADER))
        .unwrap_or(0);
    if len > capacity {
        return Err(CoreError::message(format!(
            "{} is {len} bytes but {} has room for {capacity}\n  Nothing was sent. Extend the \
             program first: solana program extend {} {}",
            plan.so_path.display(),
            plan.program_id,
            plan.program_id,
            len - capacity
        )));
    }
    Ok(())
}

/// `solana program write-buffer` into a buffer whose keypair is kept in
/// `out_dir`, so an interrupted upload resumes at the same address.
fn write_buffer(plan: &DeployPlan, payer: &Path, out_dir: &Path, verbose: bool) -> Result<Pubkey> {
    let keypair = out_dir.join("upgrade-buffer.json");
    if !keypair.is_file() {
        Keypair::generate().write(&keypair)?;
    }
    let mut cmd = Command::new("solana");
    cmd.args(["program", "write-buffer"]).arg(&plan.so_path);
    if let Some(url) = &plan.cluster {
        cmd.args(["--url", solex_types::solana_url(url)]);
    }
    cmd.arg("--keypair").arg(payer);
    cmd.arg("--buffer").arg(&keypair);
    cmd.arg("--buffer-authority").arg(payer);
    cmd.args(&plan.extra_args);
    build::run(&mut cmd, "solana program write-buffer", verbose)?;
    solana::pubkey(&keys::pubkey_of(&keypair)?)
}

/// The `vow` command: `SOLEX_VOW`, the PATH, or inside the Mac app.
pub fn vow_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SOLEX_VOW").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("vow"))
            .find(|p| p.is_file())
    });
    on_path.or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        [Some(PathBuf::from("/")), home]
            .into_iter()
            .flatten()
            .map(|root| root.join("Applications/Vow.app/Contents/Helpers/vow"))
            .find(|p| p.is_file())
    })
}

/// The network name Vow knows a cluster by. It reviews devnet and mainnet.
pub fn vow_chain(cluster: &str) -> Option<&'static str> {
    match solex_types::solana_url(cluster) {
        "mainnet-beta" => Some("mainnet"),
        "devnet" => Some("devnet"),
        _ => None,
    }
}

fn vow_new(vow: &Path, transaction: &Path, chain: &str, out_dir: &Path) -> Result<PathBuf> {
    let mut cmd = Command::new(vow);
    cmd.arg("new").arg(transaction);
    cmd.args(["--chain", chain, "--force", "--json", "--output"]);
    cmd.arg(out_dir);
    let out = build::run(&mut cmd, "vow new", false)?;
    let start = out.find('{').unwrap_or(0);
    serde_json::from_str::<serde_json::Value>(&out[start..])
        .ok()
        .and_then(|v| v["path"].as_str().map(PathBuf::from))
        .ok_or_else(|| CoreError::message(format!("vow new: unexpected output\n{out}")))
}

/// Upload the buffer and write the upgrade transaction for `authority`.
/// `out_dir` is the channel's build directory.
pub fn prepare(
    plan: &DeployPlan,
    authority: &str,
    out_dir: &Path,
    verbose: bool,
) -> Result<Handoff> {
    let cluster = plan
        .cluster
        .as_deref()
        .ok_or_else(|| CoreError::message("the channel declares no cluster"))?;
    let rpc = Rpc::new(&metadata::rpc_url(cluster));
    let program = solana::pubkey(&plan.program_id)?;
    let authority = solana::pubkey(authority)?;
    let payer_path = payer_path(plan)?;
    let payer = Keypair::read(&payer_path)?;
    check_fits(&rpc, plan, &program)?;

    let buffer = write_buffer(plan, &payer_path, out_dir, verbose)?;
    let ixs = upgrade_instructions(&program, &buffer, &payer.pubkey_bytes(), &authority)?;
    let (blockhash, _) = rpc.latest_blockhash()?;
    let message = solana::compile(&payer.pubkey_bytes(), &ixs, &blockhash);
    let wire = solana::sign_partial(&message, &[&payer]);

    let transaction = out_dir.join("upgrade.tx");
    std::fs::write(&transaction, B64.encode(&wire)).map_err(|source| CoreError::Io {
        action: "write",
        path: transaction.clone(),
        source,
    })?;

    let (review, review_skipped) = match (vow_binary(), vow_chain(cluster)) {
        (Some(vow), Some(chain)) => (Some(vow_new(&vow, &transaction, chain, out_dir)?), None),
        (None, _) => (
            None,
            Some("the `vow` command was not found (install Vow, or set SOLEX_VOW)".to_string()),
        ),
        (_, None) => (
            None,
            Some(format!("Vow reviews devnet and mainnet, not {cluster}")),
        ),
    };
    Ok(Handoff {
        buffer: solana::b58(&buffer),
        payer: payer.pubkey(),
        transaction,
        review,
        review_skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upgrade_needs_exactly_the_payer_and_the_authority() {
        let key = |n: u8| Keypair::generate().pubkey_bytes().map(|b| b ^ n);
        let (program, buffer, authority) = (key(1), key(2), key(3));
        let payer = Keypair::generate();
        let ixs =
            upgrade_instructions(&program, &buffer, &payer.pubkey_bytes(), &authority).unwrap();
        assert_eq!(ixs[0].data, 4u32.to_le_bytes());
        assert_eq!(ixs[1].data, 3u32.to_le_bytes());
        let message = solana::compile(&payer.pubkey_bytes(), &ixs, &[9; 32]);
        assert_eq!(message.signers, vec![payer.pubkey_bytes(), authority]);

        // The payer's signature is in place; the authority's slot is empty.
        let wire = solana::sign_partial(&message, &[&payer]);
        assert_eq!(wire[0], 2);
        assert_eq!(wire[1..65], payer.sign(&message.bytes));
        assert_eq!(wire[65..129], [0u8; 64]);
        assert_eq!(wire[129..], message.bytes);
        assert!(wire.len() <= solana::PACKET_DATA_SIZE);
    }

    #[test]
    fn vow_knows_two_networks() {
        assert_eq!(vow_chain("mainnet"), Some("mainnet"));
        assert_eq!(vow_chain("mainnet-beta"), Some("mainnet"));
        assert_eq!(vow_chain("devnet"), Some("devnet"));
        assert_eq!(vow_chain("localnet"), None);
        assert_eq!(vow_chain("http://127.0.0.1:8899"), None);
    }
}
