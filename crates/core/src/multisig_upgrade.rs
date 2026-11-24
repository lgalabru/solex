//! An upgrade of a program whose authority is a Squads vault, taken as far
//! as the key on this machine can take it: upload the buffer and hand it to
//! the vault, propose the upgrade, approve it, and execute it once the
//! multisig has approved.
//!
//! One run rarely finishes: other members have to vote. What was started is
//! kept next to the artifact, so the next `solex deploy` of the same build,
//! by this member or another, continues the same proposal instead of
//! uploading the program again.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    CoreError, Result,
    deploy::DeployPlan,
    handoff,
    keys::Keypair,
    metadata, program_metadata,
    solana::{self, Pubkey, Rpc},
    squads::{self, Multisig, ProposalStatus},
};

/// What a run leaves for the next one, in the channel's build directory.
const STATE_FILE: &str = "upgrade-pending.json";

/// A program account holds at most 10 MiB, so no artifact is larger.
const ARTIFACT_SIZE_MAX: u64 = 10 * 1024 * 1024;

/// How often proposing is retried when another member takes the next
/// transaction index first. Each retry is one more round of members
/// proposing at the same moment, which does not happen three times.
const PROPOSE_ATTEMPTS_MAX: u32 = 3;

/// Compute budget for executing the upgrade: the Squads program replaying
/// one loader instruction. Measured at about 30k units; the default of 200k
/// per instruction is kept explicit so the fee does not depend on it.
const EXECUTE_COMPUTE_UNITS: u32 = 200_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pending {
    buffer: String,
    /// Who paid for the buffer and gets its rent back. Part of the proposed
    /// transaction, so it is kept even when another member continues.
    spill: String,
    artifact_sha256: String,
    transaction_index: Option<u64>,
}

/// What the multisig is still waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// More members have to approve.
    Approvals,
    /// Approved; the multisig's time lock has this many seconds left.
    TimeLock { left_s: u64 },
    /// Approved, but this key may not execute.
    CannotExecute,
    /// The proposal is a draft someone has to activate.
    Draft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The program runs the new build. `signature` is the executing
    /// transaction when this run sent it.
    Upgraded {
        transaction_index: u64,
        signature: Option<String>,
    },
    Waiting {
        transaction_index: u64,
        approvals: usize,
        threshold: u16,
        blocker: Blocker,
    },
    /// The buffer is with the vault, but this key cannot propose: a member
    /// has to.
    NeedsMember,
    /// The buffer of an earlier run is gone: the multisig used or closed it.
    BufferGone,
    /// The program on the cluster already is this build. Nothing was sent.
    UpToDate,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub outcome: Outcome,
    /// Empty when there is nothing to upload.
    pub buffer: String,
    pub spill: String,
    /// The key this run acted with.
    pub member: String,
    /// Whether this run uploaded the buffer, or continued an earlier one.
    pub uploaded: bool,
    /// Bytes this run grew the program account by so the build fits.
    pub extended_bytes: usize,
}

/// Take the upgrade of `plan`'s program through the multisig as far as it
/// goes. `out_dir` is the channel's build directory.
pub fn advance(
    plan: &DeployPlan,
    reference: &squads::Reference,
    vault: &str,
    out_dir: &Path,
    verbose: bool,
) -> Result<Report> {
    let cluster = plan
        .cluster
        .as_deref()
        .ok_or_else(|| CoreError::message("the channel declares no cluster"))?;
    let rpc = Rpc::new(&metadata::rpc_url(cluster));
    let member = Keypair::read(&handoff::payer_path(plan)?)?;
    let artifact_sha256 = artifact_sha256(&plan.so_path)?;
    let state = out_dir.join(STATE_FILE);

    let previous = read(&state)?.filter(|p| p.artifact_sha256 == artifact_sha256);
    let uploaded = previous.is_none();
    let mut extended_bytes = 0;
    let mut pending = match previous {
        Some(pending) => pending,
        // A buffer handed to the vault costs rent only the multisig can
        // return, so none is made for a build that is already running.
        None if is_deployed(&rpc, &solana::pubkey(&plan.program_id)?, &plan.so_path)? => {
            return Ok(Report {
                outcome: Outcome::UpToDate,
                buffer: String::new(),
                spill: String::new(),
                member: member.pubkey(),
                uploaded: false,
                extended_bytes: 0,
            });
        }
        None => {
            let h = handoff::prepare_for_vault(plan, vault, out_dir, verbose)?;
            extended_bytes = h.extended_bytes;
            let pending = Pending {
                buffer: h.buffer,
                spill: h.payer,
                artifact_sha256,
                transaction_index: None,
            };
            write(&state, &pending)?;
            pending
        }
    };
    let upgrade = Upgrade {
        rpc: &rpc,
        multisig: solana::pubkey(&reference.multisig)?,
        vault_index: reference.vault_index,
        vault: solana::pubkey(vault)?,
        program: solana::pubkey(&plan.program_id)?,
        buffer: solana::pubkey(&pending.buffer)?,
        spill: solana::pubkey(&pending.spill)?,
        member: &member,
    };
    let outcome = upgrade.advance(&mut pending, &state)?;
    match outcome {
        // Nothing left to continue.
        Outcome::Upgraded { .. } | Outcome::BufferGone => remove(&state)?,
        Outcome::Waiting { .. } | Outcome::NeedsMember => write(&state, &pending)?,
        Outcome::UpToDate => unreachable!("decided before anything was started"),
    }
    Ok(Report {
        outcome,
        buffer: pending.buffer,
        spill: pending.spill,
        member: member.pubkey(),
        uploaded,
        extended_bytes,
    })
}

/// One upgrade and the key acting on it.
struct Upgrade<'a> {
    rpc: &'a Rpc,
    multisig: Pubkey,
    vault_index: u8,
    vault: Pubkey,
    program: Pubkey,
    buffer: Pubkey,
    spill: Pubkey,
    member: &'a Keypair,
}

impl Upgrade<'_> {
    fn advance(&self, pending: &mut Pending, state: &Path) -> Result<Outcome> {
        let multisig = squads::fetch(self.rpc, &self.multisig)?;
        if let Some(index) = pending.transaction_index {
            match squads::fetch_proposal(self.rpc, &self.multisig, index)? {
                Some(p) if p.status == ProposalStatus::Executed => {
                    return Ok(Outcome::Upgraded {
                        transaction_index: index,
                        signature: None,
                    });
                }
                // Turned down or gone: the buffer is still good, so the same
                // build is proposed again below.
                Some(p)
                    if matches!(
                        p.status,
                        ProposalStatus::Rejected | ProposalStatus::Cancelled
                    ) => {}
                None => {}
                Some(_) => return self.vote_and_execute(&multisig, index),
            }
            pending.transaction_index = None;
        }
        if handoff::buffer_authority(self.rpc, &self.buffer)? != Some(self.vault) {
            return Ok(Outcome::BufferGone);
        }
        if !multisig.can_propose(&self.member.pubkey_bytes()) {
            return Ok(Outcome::NeedsMember);
        }
        let index = self.propose(multisig.transaction_index)?;
        // Saved before anything else can fail: a run that forgot it would
        // propose the same upgrade twice.
        pending.transaction_index = Some(index);
        write(state, pending)?;
        self.vote_and_execute(&multisig, index)
    }

    fn message(&self) -> Result<squads::VaultMessage> {
        let upgrade =
            handoff::upgrade_instruction(&self.program, &self.buffer, &self.spill, &self.vault)?;
        squads::vault_message(&self.vault, &[upgrade])
    }

    /// Create the transaction and its proposal at the multisig's next index.
    fn propose(&self, mut last_index: u64) -> Result<u64> {
        let member = self.member.pubkey_bytes();
        let message = self.message()?;
        for _ in 0..PROPOSE_ATTEMPTS_MAX {
            let index = last_index
                .checked_add(1)
                .ok_or_else(|| CoreError::message("the multisig is out of transaction indexes"))?;
            let ixs = [
                squads::create_transaction(
                    &self.multisig,
                    index,
                    &member,
                    self.vault_index,
                    &message.bytes,
                ),
                squads::create_proposal(&self.multisig, index, &member),
            ];
            let sent = self.rpc.send_and_confirm(&member, &ixs, &[self.member]);
            let now = squads::fetch(self.rpc, &self.multisig)?.transaction_index;
            match sent {
                Ok(_) => return Ok(index),
                // Another member took this index first; take the next one.
                Err(_) if now != last_index => last_index = now,
                Err(e) => return Err(e),
            }
        }
        Err(CoreError::message(format!(
            "could not propose the upgrade: other members kept taking the next transaction \
             index ({PROPOSE_ATTEMPTS_MAX} attempts)"
        )))
    }

    /// Approve if this key can and has not, then execute if the multisig
    /// has approved.
    fn vote_and_execute(&self, multisig: &Multisig, index: u64) -> Result<Outcome> {
        let member = self.member.pubkey_bytes();
        let mut proposal = self.proposal(index)?;
        let can_approve = proposal.status == ProposalStatus::Active
            && multisig.can_vote(&member)
            && !proposal.approved.contains(&member);
        if can_approve {
            let ix = squads::approve(&self.multisig, index, &member);
            self.rpc.send_and_confirm(&member, &[ix], &[self.member])?;
            proposal = self.proposal(index)?;
        }
        let waiting = |blocker| {
            Ok(Outcome::Waiting {
                transaction_index: index,
                approvals: proposal.approved.len(),
                threshold: multisig.threshold,
                blocker,
            })
        };
        let approved_at_s = match proposal.status {
            ProposalStatus::Approved { timestamp_s } => timestamp_s,
            ProposalStatus::Active => return waiting(Blocker::Approvals),
            ProposalStatus::Draft => return waiting(Blocker::Draft),
            ProposalStatus::Executed => {
                return Ok(Outcome::Upgraded {
                    transaction_index: index,
                    signature: None,
                });
            }
            status @ (ProposalStatus::Rejected
            | ProposalStatus::Cancelled
            | ProposalStatus::Executing) => {
                return Err(CoreError::message(format!(
                    "proposal {index} is {status:?}; run the deploy again to propose the \
                     upgrade anew"
                )));
            }
        };
        let left_s = approved_at_s
            .saturating_add(i64::from(multisig.time_lock_s))
            .saturating_sub(now_s());
        if left_s > 0 {
            return waiting(Blocker::TimeLock {
                left_s: left_s as u64,
            });
        }
        if !multisig.can_execute(&member) {
            return waiting(Blocker::CannotExecute);
        }
        let signature = self.execute(index)?;
        Ok(Outcome::Upgraded {
            transaction_index: index,
            signature: Some(signature),
        })
    }

    fn proposal(&self, index: u64) -> Result<squads::Proposal> {
        squads::fetch_proposal(self.rpc, &self.multisig, index)?.ok_or_else(|| {
            CoreError::message(format!(
                "proposal {index} of multisig {} does not exist",
                solana::b58(&self.multisig)
            ))
        })
    }

    fn execute(&self, index: u64) -> Result<String> {
        let member = self.member.pubkey_bytes();
        let ixs = [
            solana::compute_unit_limit(EXECUTE_COMPUTE_UNITS)?,
            squads::execute(&self.multisig, index, &member, &self.message()?.accounts),
        ];
        let signature = self.rpc.send_and_confirm(&member, &ixs, &[self.member])?;
        // The buffer is consumed by the upgrade: its absence is the proof.
        let status = self.proposal(index)?.status;
        if status != ProposalStatus::Executed || self.rpc.account(&self.buffer)?.is_some() {
            return Err(CoreError::message(format!(
                "transaction {signature} confirmed, but proposal {index} is {status:?} and the \
                 upgrade did not take place"
            )));
        }
        Ok(signature)
    }
}

/// Whether the program's code on the cluster is exactly the artifact. The
/// account may be larger than the code; the rest is zeroes.
fn is_deployed(rpc: &Rpc, program: &Pubkey, artifact: &Path) -> Result<bool> {
    let address = program_metadata::program_data_address(program)?;
    let Some(account) = rpc.account(&address)? else {
        return Ok(false);
    };
    let code = account.data.get(handoff::PROGRAM_DATA_HEADER..);
    let artifact = std::fs::read(artifact).map_err(|source| CoreError::Io {
        action: "read",
        path: artifact.to_path_buf(),
        source,
    })?;
    Ok(code.is_some_and(|code| same_code(code, &artifact)))
}

fn same_code(deployed: &[u8], artifact: &[u8]) -> bool {
    match deployed.split_at_checked(artifact.len()) {
        Some((code, padding)) => code == artifact && padding.iter().all(|b| *b == 0),
        None => false,
    }
}

fn now_s() -> i64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH);
    since_epoch.map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn artifact_sha256(path: &Path) -> Result<String> {
    let io = |source| CoreError::Io {
        action: "read",
        path: path.to_path_buf(),
        source,
    };
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > ARTIFACT_SIZE_MAX {
        return Err(CoreError::message(format!(
            "{} is {len} bytes, more than a program account can hold",
            path.display()
        )));
    }
    let bytes = std::fs::read(path).map_err(io)?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn read(path: &Path) -> Result<Option<Pending>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|source| CoreError::Io {
        action: "read",
        path: path.to_path_buf(),
        source,
    })?;
    // A file this version cannot read is not worth failing a deploy over:
    // the upgrade starts again from a new buffer.
    Ok(serde_json::from_str(&text).ok())
}

fn write(path: &Path, pending: &Pending) -> Result<()> {
    let text = serde_json::to_string_pretty(pending).expect("plain data");
    std::fs::write(path, text).map_err(|source| CoreError::Io {
        action: "write",
        path: PathBuf::from(path),
        source,
    })
}

fn remove(path: &Path) -> Result<()> {
    std::fs::remove_file(path).map_err(|source| CoreError::Io {
        action: "remove",
        path: PathBuf::from(path),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_started_survives_between_runs() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join(STATE_FILE);
        assert_eq!(read(&state).unwrap(), None);
        let pending = Pending {
            buffer: solana::b58(&[1; 32]),
            spill: solana::b58(&[2; 32]),
            artifact_sha256: "ab".repeat(32),
            transaction_index: Some(11),
        };
        write(&state, &pending).unwrap();
        assert_eq!(read(&state).unwrap(), Some(pending));
        std::fs::write(&state, "not json").unwrap();
        assert_eq!(read(&state).unwrap(), None);
        remove(&state).unwrap();
        assert!(remove(&state).is_err());
    }

    #[test]
    fn deployed_code_may_be_padded_with_zeroes_only() {
        assert!(same_code(&[1, 2, 3], &[1, 2, 3]));
        assert!(same_code(&[1, 2, 3, 0, 0], &[1, 2, 3]));
        assert!(!same_code(&[1, 2, 3, 0, 4], &[1, 2, 3]));
        assert!(!same_code(&[1, 2, 4], &[1, 2, 3]));
        assert!(!same_code(&[1, 2], &[1, 2, 3]));
    }

    #[test]
    fn a_build_is_known_by_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let so = dir.path().join("program.so");
        std::fs::write(&so, b"abc").unwrap();
        let first = artifact_sha256(&so).unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.starts_with("ba7816bf"));
        std::fs::write(&so, b"abd").unwrap();
        assert_ne!(artifact_sha256(&so).unwrap(), first);
        assert!(artifact_sha256(&dir.path().join("missing.so")).is_err());
    }
}
