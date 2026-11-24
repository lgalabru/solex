//! Squads v4 multisigs as upgrade authorities.
//!
//! A multisig has two addresses that are easy to confuse: the multisig
//! account, which holds the members and the threshold, and its vault, the
//! program-derived address that signs. Only the vault can be an upgrade
//! authority; an authority handed to the multisig account is lost. The
//! manifest therefore names the multisig account and the vault is derived
//! here, never typed.

use sha2::{Digest, Sha256};

use crate::{
    CoreError, Result,
    solana::{self, AccountMeta, Instruction, Pubkey, Rpc},
};

pub const PROGRAM_ID: &str = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf";

/// Manifest prefix: `upgrade_authority = "squads:<multisig>[/<vault index>]"`.
pub const SCHEME: &str = "squads:";

/// Anchor's account discriminator, the first 8 bytes of
/// sha256("account:Multisig").
const MULTISIG_DISCRIMINATOR: [u8; 8] = [0xe0, 0x74, 0x79, 0xba, 0x44, 0xa1, 0x4f, 0xec];

/// One member: a 32-byte key and a one-byte permission mask.
const MEMBER_SIZE: usize = 33;
/// More members than any multisig a person could operate. The count comes
/// from account data, so it is bounded before anything is allocated.
const MEMBERS_MAX: usize = 1024;

const PERMISSION_INITIATE: u8 = 1 << 0;
const PERMISSION_VOTE: u8 = 1 << 1;
const PERMISSION_EXECUTE: u8 = 1 << 2;

/// A multisig named in the manifest, and which of its vaults signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub multisig: String,
    pub vault_index: u8,
}

impl Reference {
    /// Parse the part after `squads:`.
    pub fn parse(rest: &str) -> Result<Self> {
        let (multisig, index) = match rest.split_once('/') {
            Some((m, i)) => (m, Some(i)),
            None => (rest, None),
        };
        solana::pubkey(multisig).map_err(|e| {
            CoreError::message(format!(
                "`{SCHEME}{rest}`: `{multisig}` is not a multisig address ({e})"
            ))
        })?;
        let vault_index = match index {
            Some(i) => i.parse::<u8>().map_err(|_| {
                CoreError::message(format!(
                    "`{SCHEME}{rest}`: vault index `{i}` is not a number from 0 to 255"
                ))
            })?,
            None => 0,
        };
        Ok(Self {
            multisig: multisig.to_string(),
            vault_index,
        })
    }

    pub fn vault(&self) -> Result<Pubkey> {
        Ok(vault_address(
            &solana::pubkey(&self.multisig)?,
            self.vault_index,
        ))
    }
}

impl std::fmt::Display for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{SCHEME}{}", self.multisig)?;
        if self.vault_index != 0 {
            write!(f, "/{}", self.vault_index)?;
        }
        Ok(())
    }
}

pub fn program_id() -> Pubkey {
    solana::pubkey(PROGRAM_ID).expect("a valid constant")
}

/// The vault of a multisig: the address that signs what its members approve.
pub fn vault_address(multisig: &Pubkey, vault_index: u8) -> Pubkey {
    let (vault, _) = solana::find_program_address(
        &[b"multisig", multisig, b"vault", &[vault_index]],
        &program_id(),
    );
    assert!(vault != *multisig, "a vault is never its own multisig");
    vault
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub key: Pubkey,
    pub permissions: u8,
}

/// A multisig account as the cluster holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multisig {
    pub address: Pubkey,
    /// Votes needed to approve a transaction.
    pub threshold: u16,
    /// Seconds an approved transaction waits before it can run.
    pub time_lock_s: u32,
    /// A key that can change members and threshold on its own. `None` for an
    /// autonomous multisig, where such changes need a vote.
    pub config_authority: Option<Pubkey>,
    /// Index of the last transaction created; the next one takes the
    /// following index.
    pub transaction_index: u64,
    pub members: Vec<Member>,
}

impl Multisig {
    fn count(&self, permission: u8) -> usize {
        self.members
            .iter()
            .filter(|m| m.permissions & permission != 0)
            .count()
    }

    pub fn proposers(&self) -> usize {
        self.count(PERMISSION_INITIATE)
    }

    pub fn voters(&self) -> usize {
        self.count(PERMISSION_VOTE)
    }

    pub fn executors(&self) -> usize {
        self.count(PERMISSION_EXECUTE)
    }

    pub fn is_member(&self, key: &Pubkey) -> bool {
        self.members.iter().any(|m| m.key == *key)
    }

    fn can(&self, key: &Pubkey, permission: u8) -> bool {
        self.members
            .iter()
            .any(|m| m.key == *key && m.permissions & permission != 0)
    }

    pub fn can_propose(&self, key: &Pubkey) -> bool {
        self.can(key, PERMISSION_INITIATE)
    }

    pub fn can_vote(&self, key: &Pubkey) -> bool {
        self.can(key, PERMISSION_VOTE)
    }

    pub fn can_execute(&self, key: &Pubkey) -> bool {
        self.can(key, PERMISSION_EXECUTE)
    }

    /// `2 of 5`.
    pub fn describe(&self) -> String {
        format!("{} of {}", self.threshold, self.voters())
    }

    /// Why this multisig could never approve and run an upgrade, if so. The
    /// Squads program keeps these invariants itself; they are checked again
    /// because a program handed to a multisig that cannot act is lost.
    pub fn unusable(&self) -> Option<String> {
        let voters = self.voters();
        if self.threshold == 0 {
            return Some("its threshold is 0".to_string());
        }
        if usize::from(self.threshold) > voters {
            return Some(format!(
                "its threshold is {} but only {voters} members can vote",
                self.threshold
            ));
        }
        if self.proposers() == 0 {
            return Some("none of its members can propose a transaction".to_string());
        }
        if self.executors() == 0 {
            return Some("none of its members can execute a transaction".to_string());
        }
        None
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self.at.checked_add(N).filter(|e| *e <= self.data.len());
        let end = end.ok_or_else(|| CoreError::message("Squads account data is truncated"))?;
        let bytes: [u8; N] = self.data[self.at..end].try_into().expect("N bytes");
        self.at = end;
        Ok(bytes)
    }
}

/// Decode a Squads v4 `Multisig` account.
pub fn parse(address: &Pubkey, data: &[u8]) -> Result<Multisig> {
    let mut r = Reader { data, at: 0 };
    if r.take::<8>()? != MULTISIG_DISCRIMINATOR {
        return Err(CoreError::message(format!(
            "{} is owned by the Squads program but is not a multisig account",
            solana::b58(address)
        )));
    }
    let _create_key = r.take::<32>()?;
    let config_authority = Some(r.take::<32>()?).filter(|k| *k != [0; 32]);
    let threshold = u16::from_le_bytes(r.take()?);
    let time_lock_s = u32::from_le_bytes(r.take()?);
    let transaction_index = u64::from_le_bytes(r.take()?);
    let _stale_transaction_index = r.take::<8>()?;
    match r.take::<1>()? {
        [0] => {}
        [1] => {
            let _rent_collector = r.take::<32>()?;
        }
        [other] => {
            return Err(CoreError::message(format!(
                "multisig account data has an unexpected option tag {other}"
            )));
        }
    }
    let _bump = r.take::<1>()?;
    let count = u32::from_le_bytes(r.take()?) as usize;
    if count > MEMBERS_MAX || count * MEMBER_SIZE > data.len() - r.at {
        return Err(CoreError::message(format!(
            "multisig account data declares {count} members, more than it holds"
        )));
    }
    let mut members = Vec::with_capacity(count);
    for _ in 0..count {
        let key = r.take::<32>()?;
        let [permissions] = r.take::<1>()?;
        members.push(Member { key, permissions });
    }
    Ok(Multisig {
        address: *address,
        threshold,
        time_lock_s,
        config_authority,
        transaction_index,
        members,
    })
}

/// Read a multisig from the cluster. A multisig lives on the cluster it was
/// created on: the same address on another cluster is an empty account, and
/// its vault there is an address nothing can sign for.
pub fn fetch(rpc: &Rpc, multisig: &Pubkey) -> Result<Multisig> {
    let shown = solana::b58(multisig);
    let account = rpc.account(multisig)?.ok_or_else(|| {
        CoreError::message(format!(
            "multisig {shown} does not exist on this cluster\n  A Squads multisig only exists \
             on the cluster it was created on; its vault here could never sign."
        ))
    })?;
    if account.owner != program_id() {
        return Err(CoreError::message(format!(
            "{shown} is not a Squads multisig account (it is owned by {})\n  \
             `upgrade_authority = \"{SCHEME}...\"` takes the multisig account, not its vault.",
            solana::b58(&account.owner)
        )));
    }
    parse(multisig, &account.data)
}

/// Whether an account is owned by the Squads program: a multisig account or
/// one of its transactions, none of which can sign.
pub fn owns(rpc: &Rpc, address: &Pubkey) -> Result<bool> {
    Ok(rpc
        .account(address)?
        .is_some_and(|a| a.owner == program_id()))
}

/// Where the multisig keeps its transaction number `index`.
pub fn transaction_address(multisig: &Pubkey, index: u64) -> Pubkey {
    let seeds: [&[u8]; 4] = [b"multisig", multisig, b"transaction", &index.to_le_bytes()];
    solana::find_program_address(&seeds, &program_id()).0
}

/// Where the votes on transaction `index` are kept.
pub fn proposal_address(multisig: &Pubkey, index: u64) -> Pubkey {
    let seeds: [&[u8]; 5] = [
        b"multisig",
        multisig,
        b"transaction",
        &index.to_le_bytes(),
        b"proposal",
    ];
    solana::find_program_address(&seeds, &program_id()).0
}

/// Anchor's 8-byte tag: sha256("<namespace>:<name>").
fn discriminator(namespace: &str, name: &str) -> [u8; 8] {
    let hash = Sha256::digest(format!("{namespace}:{name}"));
    hash[..8].try_into().expect("8 bytes")
}

fn instruction(name: &str, accounts: Vec<AccountMeta>, args: &[u8]) -> Instruction {
    let mut data = discriminator("global", name).to_vec();
    data.extend_from_slice(args);
    Instruction {
        program_id: program_id(),
        accounts,
        data,
    }
}

/// Instructions for a vault to run, in the form the Squads program stores:
/// its own compact message, and the accounts to pass when it executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultMessage {
    pub bytes: Vec<u8>,
    /// Every account of the message in order. None is a signer: the vault
    /// signs inside the Squads program.
    pub accounts: Vec<AccountMeta>,
}

/// Compile `ixs` for `vault` to sign. Nothing else may sign: nobody is
/// there to do it when the multisig executes.
pub fn vault_message(vault: &Pubkey, ixs: &[Instruction]) -> Result<VaultMessage> {
    let order = solana::order_keys(vault, ixs);
    if order.signers != 1 {
        return Err(CoreError::message(
            "a vault transaction can only be signed by the vault",
        ));
    }
    let writable_signers = order.signers - order.readonly_signers;
    let writable_non_signers = order.keys.len() - order.signers - order.readonly_non_signers;
    // The Squads message counts accounts and instructions with one byte.
    assert!(order.keys.len() <= usize::from(u8::MAX) && ixs.len() <= usize::from(u8::MAX));

    let mut bytes = vec![
        order.signers as u8,
        writable_signers as u8,
        writable_non_signers as u8,
        order.keys.len() as u8,
    ];
    for key in &order.keys {
        bytes.extend_from_slice(key);
    }
    bytes.push(ixs.len() as u8);
    for ix in ixs {
        let data_len = u16::try_from(ix.data.len())
            .map_err(|_| CoreError::message("instruction data too long for a vault message"))?;
        bytes.push(order.index_of(&ix.program_id));
        bytes.push(ix.accounts.len() as u8);
        bytes.extend(ix.accounts.iter().map(|a| order.index_of(&a.pubkey)));
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.extend_from_slice(&ix.data);
    }
    bytes.push(0); // no address table lookups

    let accounts = order
        .keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let is_writable = i < writable_signers
                || (i >= order.signers && i - order.signers < writable_non_signers);
            match is_writable {
                true => AccountMeta::writable(*key, false),
                false => AccountMeta::readonly(*key, false),
            }
        })
        .collect();
    Ok(VaultMessage { bytes, accounts })
}

/// `vault_transaction_create`: store `message` as the multisig's next
/// transaction. `member` proposes and pays the rent.
pub fn create_transaction(
    multisig: &Pubkey,
    index: u64,
    member: &Pubkey,
    vault_index: u8,
    message: &[u8],
) -> Instruction {
    let mut args = vec![vault_index, 0]; // no ephemeral signers
    args.extend_from_slice(&(message.len() as u32).to_le_bytes());
    args.extend_from_slice(message);
    args.push(0); // no memo
    instruction(
        "vault_transaction_create",
        vec![
            AccountMeta::writable(*multisig, false),
            AccountMeta::writable(transaction_address(multisig, index), false),
            AccountMeta::readonly(*member, true),
            AccountMeta::writable(*member, true),
            AccountMeta::readonly(solana::SYSTEM_PROGRAM, false),
        ],
        &args,
    )
}

/// `proposal_create`: open transaction `index` for voting.
pub fn create_proposal(multisig: &Pubkey, index: u64, member: &Pubkey) -> Instruction {
    let mut args = index.to_le_bytes().to_vec();
    args.push(0); // active, not a draft
    instruction(
        "proposal_create",
        vec![
            AccountMeta::readonly(*multisig, false),
            AccountMeta::writable(proposal_address(multisig, index), false),
            AccountMeta::readonly(*member, true),
            AccountMeta::writable(*member, true),
            AccountMeta::readonly(solana::SYSTEM_PROGRAM, false),
        ],
        &args,
    )
}

/// `proposal_approve`: `member` votes for transaction `index`.
pub fn approve(multisig: &Pubkey, index: u64, member: &Pubkey) -> Instruction {
    instruction(
        "proposal_approve",
        vec![
            AccountMeta::readonly(*multisig, false),
            AccountMeta::writable(*member, true),
            AccountMeta::writable(proposal_address(multisig, index), false),
        ],
        &[0], // no memo
    )
}

/// `vault_transaction_execute`: run approved transaction `index`.
/// `accounts` are the stored message's, from [`vault_message`].
pub fn execute(
    multisig: &Pubkey,
    index: u64,
    member: &Pubkey,
    accounts: &[AccountMeta],
) -> Instruction {
    let mut all = vec![
        AccountMeta::readonly(*multisig, false),
        AccountMeta::writable(proposal_address(multisig, index), false),
        AccountMeta::readonly(transaction_address(multisig, index), false),
        AccountMeta::readonly(*member, true),
    ];
    all.extend_from_slice(accounts);
    instruction("vault_transaction_execute", all, &[])
}

/// Where a proposal stands. A closed set the Squads program defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatus {
    Draft,
    Active,
    Rejected,
    /// Approved at this Unix time; the time lock counts from it.
    Approved {
        timestamp_s: i64,
    },
    Executing,
    Executed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub status: ProposalStatus,
    pub approved: Vec<Pubkey>,
}

/// Decode a Squads v4 `Proposal` account.
pub fn parse_proposal(data: &[u8]) -> Result<Proposal> {
    let mut r = Reader { data, at: 0 };
    if r.take::<8>()? != discriminator("account", "Proposal") {
        return Err(CoreError::message("not a Squads proposal account"));
    }
    let _multisig = r.take::<32>()?;
    let _transaction_index = r.take::<8>()?;
    let [tag] = r.take::<1>()?;
    // Every status but the deprecated `Executing` carries a timestamp.
    let timestamp_s = match tag {
        4 => 0,
        _ => i64::from_le_bytes(r.take()?),
    };
    let status = match tag {
        0 => ProposalStatus::Draft,
        1 => ProposalStatus::Active,
        2 => ProposalStatus::Rejected,
        3 => ProposalStatus::Approved { timestamp_s },
        4 => ProposalStatus::Executing,
        5 => ProposalStatus::Executed,
        6 => ProposalStatus::Cancelled,
        other => {
            return Err(CoreError::message(format!(
                "proposal account has an unknown status {other}"
            )));
        }
    };
    let _bump = r.take::<1>()?;
    let count = u32::from_le_bytes(r.take()?) as usize;
    if count > MEMBERS_MAX || count * 32 > data.len() - r.at {
        return Err(CoreError::message(format!(
            "proposal account declares {count} approvals, more than it holds"
        )));
    }
    let mut approved = Vec::with_capacity(count);
    for _ in 0..count {
        approved.push(r.take::<32>()?);
    }
    Ok(Proposal { status, approved })
}

/// The proposal for transaction `index`, when there is one.
pub fn fetch_proposal(rpc: &Rpc, multisig: &Pubkey, index: u64) -> Result<Option<Proposal>> {
    let Some(account) = rpc.account(&proposal_address(multisig, index))? else {
        return Ok(None);
    };
    if account.owner != program_id() {
        return Ok(None);
    }
    parse_proposal(&account.data).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTISIG: &str = "HW3cHACbbZ3gmpxokmWQxywoT8xJHUnkhGAmtT1Mv7LZ";
    const VAULT: &str = "FC2HWzdiw8U5mLu9trXWPDknuhwaVnaeCQu6pQySVSt3";

    fn account(threshold: u16, permissions: &[u8], rent_collector: bool) -> Vec<u8> {
        let mut data = MULTISIG_DISCRIMINATOR.to_vec();
        data.extend_from_slice(&[1; 32]); // create_key
        data.extend_from_slice(&[0; 32]); // config_authority: none
        data.extend_from_slice(&threshold.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // time_lock
        data.extend_from_slice(&0x0102u64.to_le_bytes()); // transaction_index
        data.extend_from_slice(&[0; 8]); // stale_transaction_index
        if rent_collector {
            data.push(1);
            data.extend_from_slice(&[2; 32]);
        } else {
            data.push(0);
        }
        data.push(255); // bump
        data.extend_from_slice(&(permissions.len() as u32).to_le_bytes());
        for (i, p) in permissions.iter().enumerate() {
            data.extend_from_slice(&[i as u8 + 10; 32]);
            data.push(*p);
        }
        data.extend_from_slice(&[0; 64]); // the account's spare room
        data
    }

    #[test]
    fn the_vault_is_derived_like_the_squads_program_does() {
        // A live devnet multisig and the vault its app shows.
        let reference = Reference::parse(MULTISIG).unwrap();
        assert_eq!(reference.vault_index, 0);
        assert_eq!(solana::b58(&reference.vault().unwrap()), VAULT);
        let other = Reference::parse(&format!("{MULTISIG}/1")).unwrap();
        assert_ne!(solana::b58(&other.vault().unwrap()), VAULT);
        assert_eq!(other.to_string(), format!("squads:{MULTISIG}/1"));
        assert_eq!(reference.to_string(), format!("squads:{MULTISIG}"));
    }

    #[test]
    fn a_reference_must_be_an_address_and_a_small_index() {
        assert!(Reference::parse("not-an-address").is_err());
        assert!(Reference::parse(&format!("{MULTISIG}/256")).is_err());
        assert!(Reference::parse(&format!("{MULTISIG}/")).is_err());
    }

    #[test]
    fn a_multisig_account_decodes() {
        let address = solana::pubkey(MULTISIG).unwrap();
        for rent_collector in [false, true] {
            let m = parse(&address, &account(2, &[7, 7, 7, 1], rent_collector)).unwrap();
            assert_eq!(m.threshold, 2);
            assert_eq!(m.config_authority, None);
            assert_eq!(m.members.len(), 4);
            assert_eq!((m.proposers(), m.voters(), m.executors()), (4, 3, 3));
            assert_eq!(m.describe(), "2 of 3");
            assert!(m.is_member(&[10; 32]));
            assert!(!m.is_member(&[9; 32]));
            assert_eq!(m.unusable(), None);
            assert_eq!(m.transaction_index, 0x0102);
            assert!(m.can_propose(&[13; 32]) && !m.can_vote(&[13; 32]));
            assert!(m.can_execute(&[10; 32]) && !m.can_execute(&[9; 32]));
        }
    }

    #[test]
    fn bad_account_data_is_refused() {
        let address = solana::pubkey(MULTISIG).unwrap();
        let good = account(2, &[7, 7], false);
        assert!(parse(&address, &good[..40]).is_err());
        let mut wrong_kind = good.clone();
        wrong_kind[0] ^= 1;
        assert!(parse(&address, &wrong_kind).is_err());
        // A member count larger than the data that follows.
        let mut lying = account(2, &[7, 7], false);
        let count_at = 8 + 32 + 32 + 2 + 4 + 16 + 1 + 1;
        lying[count_at..count_at + 4].copy_from_slice(&900u32.to_le_bytes());
        assert!(parse(&address, &lying).is_err());
    }

    #[test]
    fn a_multisig_that_cannot_act_is_named() {
        let address = solana::pubkey(MULTISIG).unwrap();
        let unusable = |threshold, permissions: &[u8]| {
            parse(&address, &account(threshold, permissions, false))
                .unwrap()
                .unusable()
        };
        assert!(unusable(3, &[7, 7]).unwrap().contains("threshold is 3"));
        assert!(unusable(0, &[7, 7]).is_some());
        assert!(unusable(1, &[2, 6]).unwrap().contains("propose"));
        assert!(unusable(1, &[3, 3]).unwrap().contains("execute"));
    }

    #[test]
    fn a_vault_message_is_laid_out_as_squads_reads_it() {
        let (vault, target, program) = ([1; 32], [2; 32], [3; 32]);
        let ix = Instruction {
            program_id: program,
            accounts: vec![
                AccountMeta::writable(target, false),
                AccountMeta::readonly(vault, true),
            ],
            data: vec![7, 8, 9],
        };
        let message = vault_message(&vault, std::slice::from_ref(&ix)).unwrap();
        let mut want = vec![1, 1, 1, 3];
        want.extend_from_slice(&vault);
        want.extend_from_slice(&target);
        want.extend_from_slice(&program);
        // One instruction: program at 2, accounts [1, 0], data with a u16 length.
        want.extend_from_slice(&[1, 2, 2, 1, 0, 3, 0, 7, 8, 9, 0]);
        assert_eq!(message.bytes, want);
        let flags: Vec<(bool, bool)> = message
            .accounts
            .iter()
            .map(|a| (a.is_signer, a.is_writable))
            .collect();
        assert_eq!(flags, vec![(false, true), (false, true), (false, false)]);

        // Somebody else's signature cannot be had at execution time.
        let mut other = ix;
        other.accounts.push(AccountMeta::readonly([4; 32], true));
        assert!(vault_message(&vault, &[other]).is_err());
    }

    #[test]
    fn instructions_carry_anchor_tags_and_the_derived_accounts() {
        let (multisig, member) = ([1; 32], [2; 32]);
        let create = create_transaction(&multisig, 5, &member, 0, &[9, 9]);
        assert_eq!(
            create.data[..8],
            discriminator("global", "vault_transaction_create")
        );
        assert_eq!(create.data[8..], [0, 0, 2, 0, 0, 0, 9, 9, 0]);
        assert_eq!(create.accounts[1].pubkey, transaction_address(&multisig, 5));
        let propose = create_proposal(&multisig, 5, &member);
        assert_eq!(propose.data[8..], [5, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(propose.accounts[1].pubkey, proposal_address(&multisig, 5));
        assert_ne!(
            proposal_address(&multisig, 5),
            proposal_address(&multisig, 6)
        );
        assert_eq!(approve(&multisig, 5, &member).data.len(), 9);
        let run = execute(
            &multisig,
            5,
            &member,
            &[AccountMeta::writable([7; 32], false)],
        );
        assert_eq!(run.accounts.len(), 5);
        assert!(run.accounts[3].is_signer && run.accounts[3].pubkey == member);
    }

    #[test]
    fn a_proposal_account_decodes() {
        let account = |tag: u8, approvals: u32| {
            let mut data = discriminator("account", "Proposal").to_vec();
            data.extend_from_slice(&[1; 32]);
            data.extend_from_slice(&7u64.to_le_bytes());
            data.push(tag);
            if tag != 4 {
                data.extend_from_slice(&1_700_000_000i64.to_le_bytes());
            }
            data.push(254);
            data.extend_from_slice(&approvals.to_le_bytes());
            data.extend_from_slice(&[5; 32]);
            data.extend_from_slice(&[0; 40]);
            data
        };
        let p = parse_proposal(&account(1, 1)).unwrap();
        assert_eq!(p.status, ProposalStatus::Active);
        assert_eq!(p.approved, vec![[5; 32]]);
        let p = parse_proposal(&account(3, 1)).unwrap();
        assert_eq!(
            p.status,
            ProposalStatus::Approved {
                timestamp_s: 1_700_000_000
            }
        );
        assert_eq!(
            parse_proposal(&account(4, 0)).unwrap().status,
            ProposalStatus::Executing
        );
        assert_eq!(
            parse_proposal(&account(5, 1)).unwrap().status,
            ProposalStatus::Executed
        );
        assert!(parse_proposal(&account(9, 0)).is_err());
        assert!(parse_proposal(&account(1, 500)).is_err());
    }
}
