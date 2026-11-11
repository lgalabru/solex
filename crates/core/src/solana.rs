//! Just enough Solana client for solex: addresses and PDAs, legacy
//! transactions, and a blocking JSON-RPC client that rides out public-RPC
//! rate limits. No Solana SDK dependency; the wire formats are small and
//! stable.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{CoreError, Result, keys::Keypair};

pub type Pubkey = [u8; 32];

pub const SYSTEM_PROGRAM: Pubkey = [0; 32];
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
pub const LOADER_V3: &str = "BPFLoaderUpgradeab1e11111111111111111111111";
pub const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";

/// Largest serialized transaction the network accepts.
pub const PACKET_DATA_SIZE: usize = 1232;

pub fn pubkey(s: &str) -> Result<Pubkey> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| CoreError::message(format!("`{s}` is not base58: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| CoreError::message(format!("`{s}` is not a 32-byte address")))
}

pub fn b58(p: &Pubkey) -> String {
    bs58::encode(p).into_string()
}

fn create_program_address(seeds: &[&[u8]], program: &Pubkey) -> Option<Pubkey> {
    let mut h = Sha256::new();
    for s in seeds {
        h.update(s);
    }
    h.update(program);
    h.update(b"ProgramDerivedAddress");
    let out: Pubkey = h.finalize().into();
    // A PDA must not be a valid ed25519 point: nobody can hold its key.
    let on_curve = curve25519_dalek::edwards::CompressedEdwardsY(out)
        .decompress()
        .is_some();
    (!on_curve).then_some(out)
}

pub fn find_program_address(seeds: &[&[u8]], program: &Pubkey) -> (Pubkey, u8) {
    for bump in (0..=255u8).rev() {
        let b = [bump];
        let mut all: Vec<&[u8]> = seeds.to_vec();
        all.push(&b);
        if let Some(a) = create_program_address(&all, program) {
            return (a, bump);
        }
    }
    unreachable!("no viable bump seed")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    pub fn writable(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: true,
        }
    }
    pub fn readonly(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub program_id: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

fn compact_u16(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let mut byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

/// A compiled legacy message and the public keys that must sign it, in order.
pub struct Message {
    pub bytes: Vec<u8>,
    pub signers: Vec<Pubkey>,
}

/// Compile a legacy message: the fee payer first, then writable signers,
/// readonly signers, writable and readonly non-signers, each group in order of
/// first appearance.
pub fn compile(payer: &Pubkey, ixs: &[Instruction], blockhash: &[u8; 32]) -> Message {
    let mut order: Vec<Pubkey> = vec![*payer];
    let mut flags: HashMap<Pubkey, (bool, bool)> = HashMap::from([(*payer, (true, true))]);
    let mut add = |k: Pubkey, signer: bool, writable: bool| {
        let e = flags.entry(k).or_insert_with(|| {
            order.push(k);
            (false, false)
        });
        e.0 |= signer;
        e.1 |= writable;
    };
    for ix in ixs {
        for a in &ix.accounts {
            add(a.pubkey, a.is_signer, a.is_writable);
        }
        add(ix.program_id, false, false);
    }
    let group = |k: &Pubkey| -> u8 {
        if k == payer {
            return 0;
        }
        match flags[k] {
            (true, true) => 1,
            (true, false) => 2,
            (false, true) => 3,
            (false, false) => 4,
        }
    };
    let mut keys = order.clone();
    keys.sort_by_key(group); // stable: first appearance within a group
    let num_signers = keys.iter().filter(|k| flags[*k].0).count();
    let ro_signed = keys.iter().filter(|k| flags[*k] == (true, false)).count();
    let ro_unsigned = keys.iter().filter(|k| flags[*k] == (false, false)).count();
    let index: HashMap<Pubkey, u8> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u8))
        .collect();

    let mut out = vec![num_signers as u8, ro_signed as u8, ro_unsigned as u8];
    compact_u16(keys.len(), &mut out);
    for k in &keys {
        out.extend_from_slice(k);
    }
    out.extend_from_slice(blockhash);
    compact_u16(ixs.len(), &mut out);
    for ix in ixs {
        out.push(index[&ix.program_id]);
        compact_u16(ix.accounts.len(), &mut out);
        for a in &ix.accounts {
            out.push(index[&a.pubkey]);
        }
        compact_u16(ix.data.len(), &mut out);
        out.extend_from_slice(&ix.data);
    }
    Message {
        bytes: out,
        signers: keys[..num_signers].to_vec(),
    }
}

/// Serialized size of a transaction carrying `ixs`, signatures included.
pub fn transaction_size(payer: &Pubkey, ixs: &[Instruction]) -> usize {
    let m = compile(payer, ixs, &[0; 32]);
    let mut prefix = Vec::new();
    compact_u16(m.signers.len(), &mut prefix);
    prefix.len() + 64 * m.signers.len() + m.bytes.len()
}

/// Sign and serialize. Every required signer must be among `keypairs`.
pub fn sign(message: &Message, keypairs: &[&Keypair]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    compact_u16(message.signers.len(), &mut out);
    for s in &message.signers {
        let kp = keypairs
            .iter()
            .find(|k| k.pubkey_bytes() == *s)
            .ok_or_else(|| CoreError::message(format!("missing signer {}", b58(s))))?;
        out.extend_from_slice(&kp.sign(&message.bytes));
    }
    out.extend_from_slice(&message.bytes);
    Ok(out)
}

/// Group sequential instructions into as few transactions as fit, keeping
/// their order. `prefix` (compute budget) is repeated in every transaction.
pub fn pack(
    payer: &Pubkey,
    prefix: &[Instruction],
    ixs: Vec<Instruction>,
) -> Vec<Vec<Instruction>> {
    let mut txs: Vec<Vec<Instruction>> = Vec::new();
    let mut current: Vec<Instruction> = prefix.to_vec();
    for ix in ixs {
        current.push(ix);
        if current.len() > prefix.len() + 1 && transaction_size(payer, &current) > PACKET_DATA_SIZE
        {
            let ix = current.pop().expect("just pushed");
            txs.push(std::mem::replace(&mut current, prefix.to_vec()));
            current.push(ix);
        }
    }
    if current.len() > prefix.len() {
        txs.push(current);
    }
    txs
}

pub fn compute_unit_price(micro_lamports: u64) -> Result<Instruction> {
    let mut data = vec![3u8];
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Ok(Instruction {
        program_id: pubkey(COMPUTE_BUDGET_PROGRAM)?,
        accounts: vec![],
        data,
    })
}

pub fn compute_unit_limit(units: u32) -> Result<Instruction> {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Ok(Instruction {
        program_id: pubkey(COMPUTE_BUDGET_PROGRAM)?,
        accounts: vec![],
        data,
    })
}

pub fn transfer(from: &Pubkey, to: &Pubkey, lamports: u64) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![
            AccountMeta::writable(*from, true),
            AccountMeta::writable(*to, false),
        ],
        data,
    }
}

pub fn create_account(
    from: &Pubkey,
    new: &Pubkey,
    lamports: u64,
    space: u64,
    owner: &Pubkey,
) -> Instruction {
    let mut data = 0u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    data.extend_from_slice(&space.to_le_bytes());
    data.extend_from_slice(owner);
    Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![
            AccountMeta::writable(*from, true),
            AccountMeta::writable(*new, true),
        ],
        data,
    }
}

#[derive(Debug, Clone)]
pub struct Account {
    pub lamports: u64,
    pub owner: Pubkey,
    pub executable: bool,
    pub data: Vec<u8>,
}

/// Blocking JSON-RPC client.
pub struct Rpc {
    url: String,
    agent: ureq::Agent,
}

const MAX_ATTEMPTS: u32 = 10;
const MAX_BACKOFF: Duration = Duration::from_secs(20);

impl Rpc {
    pub fn new(url: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            url: url.to_string(),
            agent,
        }
    }

    /// One call, retried with backoff on 429 / 5xx / transport errors.
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let mut delay = Duration::from_millis(500);
        let mut last = String::new();
        for _ in 0..MAX_ATTEMPTS {
            let mut wait = delay;
            match self.agent.post(&self.url).send_json(&body) {
                Ok(mut resp) => {
                    let status = resp.status().as_u16();
                    if status == 429 || status >= 500 {
                        last = format!("HTTP {status}");
                        // Public RPCs say how long to back off.
                        if let Some(secs) = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.trim().parse::<u64>().ok())
                        {
                            wait = Duration::from_secs(secs).min(MAX_BACKOFF);
                        }
                    } else {
                        let v: Value = resp.body_mut().read_json().map_err(|e| {
                            CoreError::message(format!("{method}: invalid response ({e})"))
                        })?;
                        if let Some(err) = v.get("error") {
                            return Err(rpc_error(method, err));
                        }
                        return Ok(v["result"].clone());
                    }
                }
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(wait.max(delay));
            delay = (delay * 2).min(MAX_BACKOFF);
        }
        Err(CoreError::message(format!(
            "{method} failed after {MAX_ATTEMPTS} attempts: {last}\n  The RPC may be rate limiting; \
             pass --rpc <url> or set SOLEX_RPC_<CLUSTER> (e.g. SOLEX_RPC_DEVNET)."
        )))
    }

    pub fn account(&self, address: &Pubkey) -> Result<Option<Account>> {
        let v = self.call(
            "getAccountInfo",
            json!([b58(address), {"encoding": "base64", "commitment": "confirmed"}]),
        )?;
        let value = &v["value"];
        if value.is_null() {
            return Ok(None);
        }
        let data = value["data"][0]
            .as_str()
            .and_then(|s| B64.decode(s).ok())
            .ok_or_else(|| CoreError::message("getAccountInfo: undecodable data"))?;
        Ok(Some(Account {
            lamports: value["lamports"].as_u64().unwrap_or(0),
            owner: pubkey(value["owner"].as_str().unwrap_or_default())?,
            executable: value["executable"].as_bool().unwrap_or(false),
            data,
        }))
    }

    /// Addresses and lamports of `program`'s accounts matching `memcmp`
    /// filters (offset, bytes); data is not downloaded.
    pub fn program_accounts(
        &self,
        program: &Pubkey,
        memcmp: &[(usize, Vec<u8>)],
    ) -> Result<Vec<(Pubkey, u64)>> {
        let filters: Vec<Value> = memcmp
            .iter()
            .map(|(o, b)| json!({"memcmp": {"offset": o, "bytes": bs58::encode(b).into_string()}}))
            .collect();
        let v = self.call(
            "getProgramAccounts",
            json!([b58(program), {
                "encoding": "base64",
                "commitment": "confirmed",
                "dataSlice": {"offset": 0, "length": 0},
                "filters": filters,
            }]),
        )?;
        v.as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|it| {
                        Ok((
                            pubkey(it["pubkey"].as_str().unwrap_or_default())?,
                            it["account"]["lamports"].as_u64().unwrap_or(0),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    pub fn minimum_balance(&self, len: usize) -> Result<u64> {
        self.call("getMinimumBalanceForRentExemption", json!([len]))?
            .as_u64()
            .ok_or_else(|| CoreError::message("getMinimumBalanceForRentExemption: not a number"))
    }

    fn latest_blockhash(&self) -> Result<([u8; 32], u64)> {
        let v = self.call("getLatestBlockhash", json!([{"commitment": "confirmed"}]))?;
        let hash = pubkey(v["value"]["blockhash"].as_str().unwrap_or_default())?;
        let last = v["value"]["lastValidBlockHeight"].as_u64().unwrap_or(0);
        Ok((hash, last))
    }

    fn block_height(&self) -> Result<u64> {
        self.call("getBlockHeight", json!([{"commitment": "confirmed"}]))?
            .as_u64()
            .ok_or_else(|| CoreError::message("getBlockHeight: not a number"))
    }

    /// Sign with a fresh blockhash, send, and wait for `confirmed`. Re-signs
    /// with a new blockhash when the previous one expires unconfirmed.
    pub fn send_and_confirm(
        &self,
        payer: &Pubkey,
        ixs: &[Instruction],
        keypairs: &[&Keypair],
    ) -> Result<String> {
        for _ in 0..3 {
            let (hash, last_valid) = self.latest_blockhash()?;
            let message = compile(payer, ixs, &hash);
            let tx = sign(&message, keypairs)?;
            let sig = self
                .call(
                    "sendTransaction",
                    json!([B64.encode(&tx), {"encoding": "base64", "preflightCommitment": "confirmed"}]),
                )?
                .as_str()
                .unwrap_or_default()
                .to_string();
            let started = Instant::now();
            let mut polls = 0u32;
            loop {
                std::thread::sleep(Duration::from_millis(1500));
                polls += 1;
                let st = self.call("getSignatureStatuses", json!([[sig]]))?;
                let s = &st["value"][0];
                if !s.is_null() {
                    if !s["err"].is_null() {
                        return Err(CoreError::message(format!(
                            "transaction {sig} failed: {}",
                            s["err"]
                        )));
                    }
                    if matches!(
                        s["confirmationStatus"].as_str(),
                        Some("confirmed") | Some("finalized")
                    ) {
                        return Ok(sig);
                    }
                }
                // Expiry only needs checking now and then.
                if started.elapsed() > Duration::from_secs(120)
                    || (polls % 5 == 0 && self.block_height()? > last_valid)
                {
                    break;
                }
            }
        }
        Err(CoreError::message(
            "transaction did not confirm before its blockhash expired (3 attempts)",
        ))
    }
}

/// An RPC error, with the simulation logs when preflight rejected the
/// transaction: those carry the program's own reason.
fn rpc_error(method: &str, err: &Value) -> CoreError {
    let msg = err["message"].as_str().unwrap_or("unknown error");
    let logs: Vec<&str> = err["data"]["logs"]
        .as_array()
        .map(|l| l.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    if logs.is_empty() {
        CoreError::message(format!("{method}: {msg}"))
    } else {
        CoreError::message(format!("{method}: {msg}\n{}", logs.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_u16_matches_solana() {
        let enc = |n| {
            let mut v = Vec::new();
            compact_u16(n, &mut v);
            v
        };
        assert_eq!(enc(0), vec![0]);
        assert_eq!(enc(0x7f), vec![0x7f]);
        assert_eq!(enc(0x80), vec![0x80, 0x01]);
        assert_eq!(enc(0x3fff), vec![0xff, 0x7f]);
        assert_eq!(enc(0x4000), vec![0x80, 0x80, 0x01]);
    }

    #[test]
    fn program_data_pda_matches_the_cluster() {
        // `solana program show CHNLxBtX...` on devnet reports this ProgramData address.
        let program = pubkey("CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ").unwrap();
        let (pda, _) = find_program_address(&[&program], &pubkey(LOADER_V3).unwrap());
        assert_eq!(b58(&pda), "CDMkCSmGv4NiEmNcnxGFNdwiSwXKZCwwKdV5Px2Sz53C");
    }

    #[test]
    fn message_orders_accounts_and_counts_signers() {
        let payer = [1u8; 32];
        let other_signer = [2u8; 32];
        let ro = [3u8; 32];
        let w = [4u8; 32];
        let program = [9u8; 32];
        let ix = Instruction {
            program_id: program,
            accounts: vec![
                AccountMeta::readonly(ro, false),
                AccountMeta::writable(w, false),
                AccountMeta::readonly(other_signer, true),
            ],
            data: vec![7],
        };
        let m = compile(&payer, &[ix], &[0; 32]);
        assert_eq!(&m.bytes[..3], &[2, 1, 2]);
        assert_eq!(m.signers, vec![payer, other_signer]);
        // keys: payer, other_signer, w, ro, program
        let keys: Vec<&[u8]> = m.bytes[4..4 + 5 * 32].chunks(32).collect();
        assert_eq!(keys, vec![&payer[..], &other_signer, &w, &ro, &program]);
    }

    #[test]
    fn signed_size_matches_estimate() {
        let kp = Keypair::generate();
        let payer = kp.pubkey_bytes();
        let ixs = vec![transfer(&payer, &[5; 32], 1)];
        let m = compile(&payer, &ixs, &[0; 32]);
        assert_eq!(
            sign(&m, &[&kp]).unwrap().len(),
            transaction_size(&payer, &ixs)
        );
    }

    #[test]
    fn pack_splits_on_size_and_keeps_order() {
        let payer = [1u8; 32];
        let big = |i: u8| Instruction {
            program_id: [9; 32],
            accounts: vec![AccountMeta::writable(payer, true)],
            data: vec![i; 500],
        };
        let txs = pack(&payer, &[], (0..5).map(big).collect());
        assert_eq!(txs.len(), 3);
        let flat: Vec<u8> = txs.iter().flatten().map(|ix| ix.data[0]).collect();
        assert_eq!(flat, vec![0, 1, 2, 3, 4]);
        for tx in &txs {
            assert!(transaction_size(&payer, tx) <= PACKET_DATA_SIZE);
        }
    }
}
