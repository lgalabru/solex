//! Hand a program's upgrade authority over to the multisig the manifest
//! declares.
//!
//! The loader cannot ask a vault to sign for its own appointment, so this
//! transfer is unchecked and cannot be undone: an authority sent to an
//! address nothing can sign for is lost, and the program with it. Every
//! check here exists to make sure the destination is a vault that a working
//! multisig on this cluster controls.

use std::path::PathBuf;

use crate::{
    CoreError, Project, Result,
    authority::{self, Declared, Kind, OnChain, Signer},
    handoff,
    keys::Keypair,
    metadata, program_metadata,
    solana::{self, AccountMeta, Instruction, Pubkey, Rpc},
    squads,
};

/// One deployment of a channel: where to look, and what the manifest says
/// its authority should be.
#[derive(Debug, Clone)]
pub struct Target {
    pub channel: String,
    pub cluster: String,
    pub program_id: String,
    pub declared: Option<Declared>,
    /// The keypair that may hold the authority today: `--keypair`, else the
    /// Solana CLI's.
    pub keypair: Option<PathBuf>,
}

/// A handover that passed every check and waits for confirmation.
#[derive(Debug, Clone)]
pub struct Handover {
    pub program_id: String,
    pub cluster: String,
    pub is_mainnet: bool,
    /// The keypair holding the authority now.
    pub from_path: PathBuf,
    pub from: String,
    pub reference: squads::Reference,
    pub multisig: squads::Multisig,
    /// The multisig's vault, derived from the manifest.
    pub vault: String,
    /// Things that are legitimate but worth a second look.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Status {
    /// Nothing to hand over: the chain agrees with the manifest, or the
    /// manifest pins nothing.
    Settled(Signer),
    Pending(Box<Handover>),
}

/// Resolve the deployment of `channel` on one cluster. A handover concerns
/// one program on one cluster, so a channel with several needs `url`.
pub fn target(
    project: &Project,
    channel: &str,
    url: Option<&str>,
    keypair: Option<PathBuf>,
) -> Result<Target> {
    let channels = project.channels();
    let ch = channels.channel(channel)?;
    let cluster = match ch.cluster.select(url)?.as_slice() {
        [one] => one.clone(),
        [] => {
            return Err(CoreError::message(format!(
                "channel `{channel}` declares no cluster; pass --cluster"
            )));
        }
        several => {
            return Err(CoreError::message(format!(
                "channel `{channel}` deploys to {}; pass --cluster to pick one",
                several.join(", ")
            )));
        }
    };
    let resolved = ch.resolved(channel, Some(&cluster));
    let program_id = resolved
        .program_id
        .ok_or_else(|| CoreError::message(format!("channel `{channel}` has no program_id")))?
        .to_string();
    let declared = resolved
        .upgrade_authority
        .map(|p| authority::declared(project, p))
        .transpose()?;
    Ok(Target {
        channel: channel.to_string(),
        cluster,
        program_id,
        declared,
        keypair: keypair.or_else(metadata::solana_cli_keypair),
    })
}

/// Compare the manifest with the cluster and run every check a handover
/// needs. Sends nothing.
pub fn status(target: &Target) -> Result<Status> {
    let rpc = Rpc::new(&metadata::rpc_url(&target.cluster)).impatient();
    let signer = authority::resolve(
        &rpc,
        &target.program_id,
        target.declared.as_ref(),
        target.keypair.as_deref(),
    )?;
    if let Signer::Outgoing {
        deployed: false, ..
    } = &signer
    {
        return Err(CoreError::message(format!(
            "{} is not deployed on {} yet\n  Deploy it first (`solex deploy`); a vault cannot \
             sign a first deploy, so the program is handed over afterwards.",
            target.program_id, target.cluster
        )));
    }
    let reference = match &target.declared {
        Some(Declared::Squads(r)) => Some(r),
        _ => None,
    };
    Ok(
        match pending(&target.program_id, &target.cluster, reference, &signer)? {
            Some(handover) => Status::Pending(Box::new(handover)),
            None => Status::Settled(signer),
        },
    )
}

/// The handover `signer` still owes, checked: `None` when the authority is
/// where the manifest says. A deploy calls this before uploading anything,
/// so a destination that would be refused is refused first.
pub fn pending(
    program_id: &str,
    cluster: &str,
    reference: Option<&squads::Reference>,
    signer: &Signer,
) -> Result<Option<Handover>> {
    let Signer::Outgoing {
        path,
        address,
        to,
        kind,
        ..
    } = signer
    else {
        return Ok(None);
    };
    let multisig = verified_multisig(to, kind.clone())?;
    let reference =
        reference.unwrap_or_else(|| unreachable!("only a declared multisig resolves to a vault"));
    assert_eq!(solana::b58(&reference.vault()?), *to);
    let from = solana::pubkey(address)?;
    let warnings = warnings(&multisig, &from);
    Ok(Some(Handover {
        program_id: program_id.to_string(),
        is_mainnet: solex_types::solana_url(cluster) == "mainnet-beta",
        cluster: cluster.to_string(),
        from_path: path.clone(),
        from: address.clone(),
        reference: reference.clone(),
        multisig,
        vault: to.clone(),
        warnings,
    }))
}

/// Only a vault whose multisig was read from this cluster is a destination.
/// Everything else is refused with what to declare instead.
fn verified_multisig(to: &str, kind: Kind) -> Result<squads::Multisig> {
    let refuse = |why: String, next: String| {
        Err(CoreError::message(format!(
            "{why}\n  Nothing was sent. {next}"
        )))
    };
    match kind {
        Kind::Squads(multisig) => match multisig.unusable() {
            None => Ok(multisig),
            Some(why) => refuse(
                format!(
                    "multisig {} could never approve an upgrade: {why}",
                    solana::b58(&multisig.address)
                ),
                "A program handed to it would be frozen.".to_string(),
            ),
        },
        Kind::Derived { owner: Some(owner) } if owner == squads::PROGRAM_ID => refuse(
            format!(
                "{to} is a Squads multisig account, not its vault: it cannot sign, and an \
                 authority handed to it is lost"
            ),
            format!(
                "Declare `upgrade_authority = \"{}{to}\"`; solex derives the vault.",
                squads::SCHEME
            ),
        ),
        Kind::Derived { .. } => refuse(
            format!(
                "{to} is a program-derived address, and nothing here shows that anything can \
                 sign for it"
            ),
            format!(
                "If it is the vault of a Squads multisig, declare `upgrade_authority = \
                 \"{}<multisig account>\"` so the vault is derived and checked instead of typed.",
                squads::SCHEME
            ),
        ),
        Kind::Key => refuse(
            format!("{to} is an ordinary key"),
            "Handing a program to a key takes that key's signature, so that a mistyped address \
             cannot receive it; that is not built yet."
                .to_string(),
        ),
    }
}

fn warnings(multisig: &squads::Multisig, from: &Pubkey) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(key) = &multisig.config_authority {
        out.push(format!(
            "{} can replace the members and the threshold on its own (config authority), so \
             it alone ends up controlling the program.",
            solana::b58(key)
        ));
    }
    if multisig.threshold == 1 {
        out.push("A threshold of 1 lets any single voting member upgrade the program.".into());
    }
    if !multisig.is_member(from) {
        out.push(format!(
            "{} is not a member: after the handover this key can neither propose nor approve \
             an upgrade.",
            solana::b58(from)
        ));
    }
    if multisig.time_lock_s > 0 {
        out.push(format!(
            "Approved upgrades wait {} seconds before they can run (time lock).",
            multisig.time_lock_s
        ));
    }
    out
}

/// `SetAuthority` on the program. The new authority is always present:
/// leaving it out would make the program immutable.
fn set_authority(program: &Pubkey, current: &Pubkey, new: &Pubkey) -> Result<Instruction> {
    assert!(current != new, "handing an authority to itself");
    handoff::loader_ix(
        4,
        vec![
            AccountMeta::writable(program_metadata::program_data_address(program)?, false),
            AccountMeta::readonly(*current, true),
            AccountMeta::readonly(*new, false),
        ],
    )
}

/// Send the handover. `confirmed_vault` is the vault address as the person
/// read it somewhere other than this manifest (the Squads app, a teammate):
/// it must be the one derived here.
pub fn execute(handover: &Handover, confirmed_vault: &str) -> Result<String> {
    if confirmed_vault.trim() != handover.vault {
        return Err(CoreError::message(format!(
            "`{}` is not the vault of multisig {} on {}, which is {}\n  Nothing was sent. \
             Check the multisig address in the manifest against the Squads app.",
            confirmed_vault.trim(),
            handover.reference.multisig,
            handover.cluster,
            handover.vault
        )));
    }
    let rpc = Rpc::new(&metadata::rpc_url(&handover.cluster));
    let program = solana::pubkey(&handover.program_id)?;
    let vault = solana::pubkey(&handover.vault)?;
    let signer = Keypair::read(&handover.from_path)?;
    let current = signer.pubkey_bytes();
    // Read again right before sending: the checks may be minutes old.
    if authority::on_chain(&rpc, &program)? != OnChain::Upgradeable(current) {
        return Err(CoreError::message(format!(
            "{} is no longer controlled by {}\n  Nothing was sent.",
            handover.program_id, handover.from
        )));
    }
    let ix = set_authority(&program, &current, &vault)?;
    let signature = rpc.send_and_confirm(&current, &[ix], &[&signer])?;
    match authority::on_chain(&rpc, &program)? {
        OnChain::Upgradeable(now) if now == vault => Ok(signature),
        other => Err(CoreError::message(format!(
            "transaction {signature} confirmed, but the cluster reports {other:?} as the \
             authority of {} instead of {}",
            handover.program_id, handover.vault
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn multisig(threshold: u16, members: &[(Pubkey, u8)]) -> squads::Multisig {
        squads::Multisig {
            address: [9; 32],
            threshold,
            time_lock_s: 0,
            config_authority: None,
            transaction_index: 0,
            members: members
                .iter()
                .map(|(key, permissions)| squads::Member {
                    key: *key,
                    permissions: *permissions,
                })
                .collect(),
        }
    }

    #[test]
    fn only_a_working_multisig_vault_is_a_destination() {
        let to = solana::b58(&[5; 32]);
        let good = multisig(2, &[([1; 32], 7), ([2; 32], 7)]);
        assert!(verified_multisig(&to, Kind::Squads(good)).is_ok());

        let frozen = multisig(3, &[([1; 32], 7), ([2; 32], 7)]);
        let err = verified_multisig(&to, Kind::Squads(frozen)).unwrap_err();
        assert!(err.to_string().contains("could never approve"), "{err}");

        let account = Kind::Derived {
            owner: Some(squads::PROGRAM_ID.to_string()),
        };
        let err = verified_multisig(&to, account).unwrap_err().to_string();
        assert!(err.contains("not its vault"), "{err}");
        assert!(err.contains(&format!("squads:{to}")), "{err}");

        let unknown = Kind::Derived { owner: None };
        assert!(verified_multisig(&to, unknown).is_err());
        assert!(verified_multisig(&to, Kind::Key).is_err());
    }

    #[test]
    fn warnings_cover_what_changes_who_is_in_control() {
        let me = [1; 32];
        let mut m = multisig(2, &[(me, 7), ([2; 32], 7)]);
        assert!(warnings(&m, &me).is_empty());
        assert_eq!(warnings(&m, &[3; 32]).len(), 1);
        m.config_authority = Some([4; 32]);
        m.threshold = 1;
        m.time_lock_s = 60;
        assert_eq!(warnings(&m, &me).len(), 3);
    }

    #[test]
    fn the_new_authority_is_always_named() {
        let (program, current, vault) = ([1; 32], [2; 32], [3; 32]);
        let ix = set_authority(&program, &current, &vault).unwrap();
        assert_eq!(ix.data, 4u32.to_le_bytes());
        assert_eq!(ix.accounts.len(), 3);
        assert!(ix.accounts[0].is_writable && !ix.accounts[0].is_signer);
        assert!(ix.accounts[1].is_signer && ix.accounts[1].pubkey == current);
        assert!(!ix.accounts[2].is_signer && ix.accounts[2].pubkey == vault);
    }

    #[test]
    fn the_vault_typed_by_hand_must_be_the_derived_one() {
        let reference = squads::Reference::parse(&solana::b58(&[9; 32])).unwrap();
        let handover = Handover {
            program_id: solana::b58(&[1; 32]),
            cluster: "devnet".into(),
            is_mainnet: false,
            from_path: PathBuf::from("missing.json"),
            from: solana::b58(&[2; 32]),
            vault: solana::b58(&reference.vault().unwrap()),
            reference,
            multisig: multisig(1, &[([2; 32], 7)]),
            warnings: Vec::new(),
        };
        // Refused before the keypair is read or the network is touched.
        let err = execute(&handover, &solana::b58(&[9; 32])).unwrap_err();
        assert!(err.to_string().contains("is not the vault"), "{err}");
    }
}
