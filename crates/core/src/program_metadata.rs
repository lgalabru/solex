//! Client for the Program Metadata program
//! (`ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S`,
//! <https://github.com/solana-program/program-metadata>), ported from its JS
//! client so solex can publish IDLs without the `program-metadata` CLI.
//!
//! Only canonical metadata (signed by the program's upgrade authority) with
//! directly stored data is supported, which is what an IDL upload needs.
//! Writes follow the JS client: a missing account is created through a
//! buffer at the metadata PDA (`allocate`, `write`, `initialize`); a buffer
//! left there by an interrupted create is resumed; an existing account is
//! updated through a temporary buffer (`set_data`), then trimmed if smaller.

use std::io::{Read, Write as _};

use crate::{
    CoreError, Result,
    keys::Keypair,
    solana::{self, AccountMeta, Instruction, Pubkey, Rpc, b58},
};

pub const PROGRAM_ID: &str = "ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S";
pub const HEADER_LEN: usize = 96;
pub const SEED_LEN: usize = 16;
/// Most an account may grow within one instruction.
pub const REALLOC_LIMIT: usize = 10_240;
/// Data bytes per `write`, leaving room for the rest of the transaction.
const WRITE_CHUNK: usize = 900;

pub mod discriminator {
    pub const EMPTY: u8 = 0;
    pub const BUFFER: u8 = 1;
    pub const METADATA: u8 = 2;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Encoding {
    None = 0,
    Utf8 = 1,
    Base58 = 2,
    Base64 = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    None = 0,
    Gzip = 1,
    Zlib = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Format {
    None = 0,
    Json = 1,
    Yaml = 2,
    Toml = 3,
}

impl Format {
    pub fn parse(s: Option<&str>) -> Format {
        match s {
            Some("json") => Format::Json,
            Some("yaml") => Format::Yaml,
            Some("toml") => Format::Toml,
            _ => Format::None,
        }
    }
}

const DATA_SOURCE_DIRECT: u8 = 0;

/// The 96-byte header shared by metadata and buffer accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub discriminator: u8,
    pub program: Pubkey,
    pub authority: Option<Pubkey>,
    pub mutable: bool,
    pub canonical: bool,
    pub seed: [u8; SEED_LEN],
    pub encoding: u8,
    pub compression: u8,
    pub format: u8,
    pub data_source: u8,
    pub data_length: u32,
}

impl Header {
    pub fn parse(data: &[u8]) -> Option<Header> {
        if data.len() < HEADER_LEN {
            return None;
        }
        let key = |o: usize| -> Pubkey { data[o..o + 32].try_into().expect("32 bytes") };
        let authority = key(33);
        Some(Header {
            discriminator: data[0],
            program: key(1),
            authority: (authority != [0; 32]).then_some(authority),
            mutable: data[65] != 0,
            canonical: data[66] != 0,
            seed: data[67..83].try_into().expect("16 bytes"),
            encoding: data[83],
            compression: data[84],
            format: data[85],
            data_source: data[86],
            data_length: u32::from_le_bytes(data[87..91].try_into().expect("4 bytes")),
        })
    }
}

pub fn seed_bytes(seed: &str) -> Result<[u8; SEED_LEN]> {
    let b = seed.as_bytes();
    if b.is_empty() || b.len() > SEED_LEN {
        return Err(CoreError::message(format!(
            "metadata seed `{seed}` must be 1 to {SEED_LEN} bytes"
        )));
    }
    let mut out = [0u8; SEED_LEN];
    out[..b.len()].copy_from_slice(b);
    Ok(out)
}

fn id() -> Pubkey {
    solana::pubkey(PROGRAM_ID).expect("valid program id")
}

/// Canonical metadata PDA: `[program, seed]`.
pub fn canonical_address(program: &Pubkey, seed: &str) -> Result<Pubkey> {
    Ok(solana::find_program_address(&[program, &seed_bytes(seed)?], &id()).0)
}

pub fn program_data_address(program: &Pubkey) -> Result<Pubkey> {
    Ok(solana::find_program_address(&[program], &solana::pubkey(solana::LOADER_V3)?).0)
}

pub fn pack(content: &[u8], compression: Compression) -> Result<Vec<u8>> {
    let io = |e: std::io::Error| CoreError::message(format!("compression failed: {e}"));
    Ok(match compression {
        Compression::None => content.to_vec(),
        Compression::Zlib => {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(content).map_err(io)?;
            e.finish().map_err(io)?
        }
        Compression::Gzip => {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(content).map_err(io)?;
            e.finish().map_err(io)?
        }
    })
}

/// Decode stored data back to the uploaded content.
pub fn unpack(header: &Header, data: &[u8]) -> Result<Vec<u8>> {
    let io = |e: std::io::Error| CoreError::message(format!("decompression failed: {e}"));
    let mut out = Vec::new();
    match header.compression {
        0 => out = data.to_vec(),
        1 => {
            flate2::read::GzDecoder::new(data)
                .read_to_end(&mut out)
                .map_err(io)?;
        }
        2 => {
            flate2::read::ZlibDecoder::new(data)
                .read_to_end(&mut out)
                .map_err(io)?;
        }
        c => return Err(CoreError::message(format!("unknown compression {c}"))),
    }
    Ok(match header.encoding {
        2 => bs58::decode(&out)
            .into_vec()
            .map_err(|e| CoreError::message(format!("base58: {e}")))?,
        3 => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(&out)
                .map_err(|e| CoreError::message(format!("base64: {e}")))?
        }
        _ => out,
    })
}

// ---------- instructions ----------

struct Accounts {
    authority: Pubkey,
    program: Pubkey,
    program_data: Pubkey,
}

fn allocate_pda(target: &Pubkey, a: &Accounts, seed: &[u8; SEED_LEN]) -> Instruction {
    let mut data = vec![7u8];
    data.extend_from_slice(seed);
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*target, false),
            AccountMeta::readonly(a.authority, true),
            AccountMeta::readonly(a.program, false),
            AccountMeta::readonly(a.program_data, false),
            AccountMeta::readonly(solana::SYSTEM_PROGRAM, false),
        ],
        data,
    }
}

/// `allocate` for a keypair buffer: the buffer signs as its own authority.
fn allocate_keypair(buffer: &Pubkey) -> Instruction {
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*buffer, false),
            AccountMeta::readonly(*buffer, true),
            AccountMeta::readonly(id(), false),
            AccountMeta::readonly(id(), false),
            AccountMeta::readonly(id(), false),
        ],
        data: vec![7u8],
    }
}

fn set_authority(account: &Pubkey, current: &Pubkey, new: &Pubkey) -> Instruction {
    let mut data = vec![2u8, 1];
    data.extend_from_slice(new);
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*account, false),
            AccountMeta::readonly(*current, true),
            AccountMeta::readonly(id(), false),
            AccountMeta::readonly(id(), false),
        ],
        data,
    }
}

fn extend(account: &Pubkey, a: &Accounts, length: u16) -> Instruction {
    let mut data = vec![8u8];
    data.extend_from_slice(&length.to_le_bytes());
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*account, false),
            AccountMeta::readonly(a.authority, true),
            AccountMeta::readonly(a.program, false),
            AccountMeta::readonly(a.program_data, false),
        ],
        data,
    }
}

fn extends(account: &Pubkey, a: &Accounts, mut total: usize) -> Vec<Instruction> {
    let mut out = Vec::new();
    while total > 0 {
        let n = total.min(REALLOC_LIMIT);
        out.push(extend(account, a, n as u16));
        total -= n;
    }
    out
}

fn writes(buffer: &Pubkey, authority: &Pubkey, data: &[u8]) -> Vec<Instruction> {
    data.chunks(WRITE_CHUNK)
        .enumerate()
        .map(|(i, chunk)| {
            let mut d = vec![0u8];
            d.extend_from_slice(&((i * WRITE_CHUNK) as u32).to_le_bytes());
            d.extend_from_slice(chunk);
            Instruction {
                program_id: id(),
                accounts: vec![
                    AccountMeta::writable(*buffer, false),
                    AccountMeta::readonly(*authority, true),
                    AccountMeta::readonly(id(), false),
                ],
                data: d,
            }
        })
        .collect()
}

fn data_args(encoding: Encoding, compression: Compression, format: Format) -> [u8; 4] {
    [
        encoding as u8,
        compression as u8,
        format as u8,
        DATA_SOURCE_DIRECT,
    ]
}

/// `initialize` from the buffer already at the metadata PDA.
fn initialize(
    metadata: &Pubkey,
    a: &Accounts,
    seed: &[u8; SEED_LEN],
    args: [u8; 4],
) -> Instruction {
    let mut data = vec![1u8];
    data.extend_from_slice(seed);
    data.extend_from_slice(&args);
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*metadata, false),
            AccountMeta::readonly(a.authority, true),
            AccountMeta::readonly(a.program, false),
            AccountMeta::readonly(a.program_data, false),
            AccountMeta::readonly(id(), false),
        ],
        data,
    }
}

fn set_data(metadata: &Pubkey, a: &Accounts, buffer: &Pubkey, args: [u8; 4]) -> Instruction {
    let mut data = vec![3u8];
    data.extend_from_slice(&args);
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*metadata, false),
            AccountMeta::readonly(a.authority, true),
            AccountMeta::writable(*buffer, false),
            AccountMeta::readonly(a.program, false),
            AccountMeta::readonly(a.program_data, false),
        ],
        data,
    }
}

fn close_keypair_buffer(buffer: &Pubkey, authority: &Pubkey, destination: &Pubkey) -> Instruction {
    Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*buffer, false),
            AccountMeta::readonly(*authority, true),
            AccountMeta::readonly(id(), false),
            AccountMeta::readonly(id(), false),
            AccountMeta::writable(*destination, false),
        ],
        data: vec![6u8],
    }
}

fn trim(account: &Pubkey, a: &Accounts, destination: &Pubkey) -> Result<Instruction> {
    Ok(Instruction {
        program_id: id(),
        accounts: vec![
            AccountMeta::writable(*account, false),
            AccountMeta::readonly(a.authority, true),
            AccountMeta::readonly(a.program, false),
            AccountMeta::readonly(a.program_data, false),
            AccountMeta::writable(*destination, false),
            AccountMeta::readonly(solana::pubkey(solana::RENT_SYSVAR)?, false),
        ],
        data: vec![5u8],
    })
}

// ---------- reads ----------

/// What the metadata PDA holds.
#[derive(Debug, Clone)]
pub enum State {
    Missing,
    /// A buffer left by an interrupted create, with what it holds so far.
    PreAllocated {
        lamports: u64,
        data: Vec<u8>,
    },
    Initialized {
        header: Header,
        lamports: u64,
        data: Vec<u8>,
    },
}

pub fn state(rpc: &Rpc, metadata: &Pubkey) -> Result<State> {
    let Some(acc) = rpc.account(metadata)? else {
        return Ok(State::Missing);
    };
    if acc.owner != id() || acc.data.is_empty() {
        return Ok(State::Missing);
    }
    let header = Header::parse(&acc.data)
        .ok_or_else(|| CoreError::message(format!("{} is too small", b58(metadata))))?;
    let body = acc.data[HEADER_LEN..].to_vec();
    Ok(match header.discriminator {
        discriminator::BUFFER => State::PreAllocated {
            lamports: acc.lamports,
            data: body,
        },
        discriminator::METADATA => {
            let len = (header.data_length as usize).min(body.len());
            let len = if len == 0 { body.len() } else { len };
            State::Initialized {
                lamports: acc.lamports,
                data: body[..len].to_vec(),
                header,
            }
        }
        _ => State::Missing,
    })
}

/// The canonical metadata content for `seed`, decoded; `None` when there is
/// no initialized account.
pub fn fetch(rpc: &Rpc, program: &Pubkey, seed: &str) -> Result<Option<Vec<u8>>> {
    match state(rpc, &canonical_address(program, seed)?)? {
        State::Initialized { header, data, .. } => Ok(Some(unpack(&header, &data)?)),
        _ => Ok(None),
    }
}

/// The program's upgrade authority, read from its ProgramData account.
pub fn upgrade_authority(rpc: &Rpc, program: &Pubkey) -> Result<Option<Pubkey>> {
    let acc = rpc
        .account(program)?
        .ok_or_else(|| CoreError::message(format!("program {} does not exist", b58(program))))?;
    if !acc.executable || acc.owner != solana::pubkey(solana::LOADER_V3)? {
        return Err(CoreError::message(format!(
            "{} is not an upgradeable (loader v3) program",
            b58(program)
        )));
    }
    let pd = rpc
        .account(&program_data_address(program)?)?
        .ok_or_else(|| CoreError::message("program data account missing"))?;
    // u32 discriminator (3), u64 slot, Option<Pubkey>
    if pd.data.len() < 13 || pd.data[..4] != 3u32.to_le_bytes() {
        return Err(CoreError::message("unexpected program data layout"));
    }
    Ok((pd.data[12] == 1 && pd.data.len() >= 45)
        .then(|| pd.data[13..45].try_into().expect("32 bytes")))
}

/// Buffer accounts `authority` controls (keypair buffers and pre-allocated
/// PDAs alike), with the lamports each holds.
pub fn buffers(rpc: &Rpc, authority: &Pubkey) -> Result<Vec<(Pubkey, u64)>> {
    rpc.program_accounts(
        &id(),
        &[(0, vec![discriminator::BUFFER]), (33, authority.to_vec())],
    )
}

/// Close `buffers` and send their rent to the payer. The buffer's explicit
/// authority is enough for the program, so no program accounts are needed.
pub fn close_buffers(
    rpc: &Rpc,
    buffers: &[Pubkey],
    authority: &Keypair,
    payer: &Keypair,
    priority_fee: Option<u64>,
) -> Result<Vec<String>> {
    let a = authority.pubkey_bytes();
    let p = payer.pubkey_bytes();
    let mut prefix = Vec::new();
    if let Some(fee) = priority_fee {
        prefix.push(solana::compute_unit_price(fee)?);
    }
    let ixs = buffers
        .iter()
        .map(|b| close_keypair_buffer(b, &a, &p))
        .collect();
    let mut sigs = Vec::new();
    for tx in solana::pack(&p, &prefix, ixs) {
        sigs.push(rpc.send_and_confirm(&p, &tx, &[payer, authority])?);
    }
    Ok(sigs)
}

// ---------- writes ----------

pub struct WriteRequest<'a> {
    pub program: Pubkey,
    pub seed: &'a str,
    pub content: &'a [u8],
    pub format: Format,
    pub authority: &'a Keypair,
    pub payer: &'a Keypair,
    pub priority_fee: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct WriteReport {
    pub metadata: Pubkey,
    /// `created`, `resumed` or `updated`.
    pub action: &'static str,
    pub signatures: Vec<String>,
}

/// Create or update the canonical metadata account for `req.seed`.
pub fn write(rpc: &Rpc, req: &WriteRequest) -> Result<WriteReport> {
    let authority = req.authority.pubkey_bytes();
    let payer = req.payer.pubkey_bytes();
    match upgrade_authority(rpc, &req.program)? {
        Some(a) if a == authority => {}
        Some(a) => {
            return Err(CoreError::message(format!(
                "{} is not the upgrade authority of {} ({} is); canonical metadata must be \
                 signed by it\n  Pass --keypair <upgrade authority keypair>.",
                b58(&authority),
                b58(&req.program),
                b58(&a)
            )));
        }
        None => {
            return Err(CoreError::message(format!(
                "{} is immutable (no upgrade authority); its canonical metadata cannot be written",
                b58(&req.program)
            )));
        }
    }

    let seed = seed_bytes(req.seed)?;
    let metadata = canonical_address(&req.program, req.seed)?;
    let accounts = Accounts {
        authority,
        program: req.program,
        program_data: program_data_address(&req.program)?,
    };
    let compression = Compression::Zlib;
    let packed = pack(req.content, compression)?;
    let args = data_args(Encoding::Utf8, compression, req.format);

    let mut prefix = Vec::new();
    if let Some(fee) = req.priority_fee {
        prefix.push(solana::compute_unit_limit(200_000)?);
        prefix.push(solana::compute_unit_price(fee)?);
    }

    let st = state(rpc, &metadata)?;
    let mut temp_buffer: Option<Keypair> = None;
    let (action, ixs) = match st {
        State::Missing | State::PreAllocated { .. } => {
            let (allocated, lamports, existing) = match &st {
                State::PreAllocated { lamports, data } => (true, *lamports, data.clone()),
                _ => (false, 0, Vec::new()),
            };
            if existing.len() > packed.len() {
                return Err(CoreError::message(format!(
                    "{} holds a {}-byte buffer from an interrupted create, larger than the {} bytes \
                     to store, and buffers cannot shrink; close it and write again",
                    b58(&metadata),
                    existing.len(),
                    packed.len()
                )));
            }
            let mut ixs = Vec::new();
            let rent = rpc.minimum_balance(HEADER_LEN + packed.len())?;
            if rent > lamports {
                ixs.push(solana::transfer(&payer, &metadata, rent - lamports));
            }
            if !allocated {
                ixs.push(allocate_pda(&metadata, &accounts, &seed));
            }
            let growth = if allocated { 0 } else { HEADER_LEN } + packed.len() - existing.len();
            if growth > REALLOC_LIMIT {
                ixs.extend(extends(&metadata, &accounts, packed.len() - existing.len()));
            }
            if existing != packed {
                ixs.extend(writes(&metadata, &authority, &packed));
            }
            ixs.push(initialize(&metadata, &accounts, &seed, args));
            (if allocated { "resumed" } else { "created" }, ixs)
        }
        State::Initialized { header, data, .. } => {
            if !header.mutable {
                return Err(CoreError::message(format!(
                    "{} is immutable",
                    b58(&metadata)
                )));
            }
            let diff = packed.len() as i64 - data.len() as i64;
            let mut ixs = Vec::new();
            if diff > 0 {
                let extra = rpc.minimum_balance(diff as usize)? - rpc.minimum_balance(0)?;
                ixs.push(solana::transfer(&payer, &metadata, extra));
                if diff as usize > REALLOC_LIMIT {
                    ixs.extend(extends(&metadata, &accounts, diff as usize));
                }
            }
            let buffer = Keypair::generate();
            let b = buffer.pubkey_bytes();
            let space = HEADER_LEN + packed.len();
            ixs.push(solana::create_account(
                &payer,
                &b,
                rpc.minimum_balance(space)?,
                space as u64,
                &id(),
            ));
            ixs.push(allocate_keypair(&b));
            ixs.push(set_authority(&b, &b, &authority));
            ixs.extend(writes(&b, &authority, &packed));
            ixs.push(set_data(&metadata, &accounts, &b, args));
            ixs.push(close_keypair_buffer(&b, &authority, &payer));
            if diff < 0 {
                ixs.push(trim(&metadata, &accounts, &payer)?);
            }
            temp_buffer = Some(buffer);
            ("updated", ixs)
        }
    };

    let mut keypairs: Vec<&Keypair> = vec![req.payer, req.authority];
    if let Some(b) = &temp_buffer {
        keypairs.push(b);
    }
    let mut signatures = Vec::new();
    for tx in solana::pack(&payer, &prefix, ixs) {
        match rpc.send_and_confirm(&payer, &tx, &keypairs) {
            Ok(sig) => signatures.push(sig),
            Err(e) => {
                // A temporary buffer only holds rent; try to give it back.
                let mut leftover = None;
                if let Some(b) = &temp_buffer {
                    let bp = b.pubkey_bytes();
                    if rpc.account(&bp).ok().flatten().is_some() {
                        let mut close = prefix.clone();
                        close.push(close_keypair_buffer(&bp, &authority, &payer));
                        if rpc.send_and_confirm(&payer, &close, &keypairs).is_err() {
                            leftover = Some(b58(&bp));
                        }
                    }
                }
                let next = if temp_buffer.is_some() {
                    "writing again starts a new update"
                } else {
                    "writing again resumes from the buffer at the metadata address"
                };
                let mut msg = format!(
                    "{e}\n  {} transaction(s) landed before the failure; {next}.",
                    signatures.len()
                );
                if let Some(b) = leftover {
                    msg.push_str(&format!(
                        "\n  Temporary buffer {b} still holds rent; reclaim it with \
                         `solex metadata --close-buffers`."
                    ));
                }
                return Err(CoreError::message(msg));
            }
        }
    }
    Ok(WriteReport {
        metadata,
        action,
        signatures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_pda_matches_the_cli() {
        // `program-metadata write idl CHNLxBtX...` reported this account.
        let program = solana::pubkey("CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ").unwrap();
        assert_eq!(
            b58(&canonical_address(&program, "idl").unwrap()),
            "H4vafhd6PQm6XMbo3qJdBuk8njHgqvZNCu2TZDhH45uc"
        );
    }

    #[test]
    fn header_roundtrip_offsets() {
        let mut d = vec![0u8; HEADER_LEN + 3];
        d[0] = discriminator::METADATA;
        d[1..33].copy_from_slice(&[7; 32]);
        d[33..65].copy_from_slice(&[8; 32]);
        d[65] = 1;
        d[66] = 1;
        d[67..70].copy_from_slice(b"idl");
        d[83] = 1;
        d[84] = 2;
        d[85] = 1;
        d[87..91].copy_from_slice(&3u32.to_le_bytes());
        let h = Header::parse(&d).unwrap();
        assert_eq!(h.program, [7; 32]);
        assert_eq!(h.authority, Some([8; 32]));
        assert!(h.mutable && h.canonical);
        assert_eq!(&h.seed[..3], b"idl");
        assert_eq!((h.encoding, h.compression, h.format), (1, 2, 1));
        assert_eq!(h.data_length, 3);
    }

    #[test]
    fn zlib_roundtrip() {
        let content = br#"{"kind":"rootNode"}"#;
        let packed = pack(content, Compression::Zlib).unwrap();
        let h = Header {
            discriminator: 2,
            program: [0; 32],
            authority: None,
            mutable: true,
            canonical: true,
            seed: [0; 16],
            encoding: 1,
            compression: 2,
            format: 1,
            data_source: 0,
            data_length: packed.len() as u32,
        };
        assert_eq!(unpack(&h, &packed).unwrap(), content);
    }

    #[test]
    fn writes_chunk_with_offsets() {
        let ixs = writes(&[1; 32], &[2; 32], &vec![9u8; 2000]);
        assert_eq!(ixs.len(), 3);
        assert_eq!(&ixs[1].data[1..5], &900u32.to_le_bytes());
        assert_eq!(ixs[2].data.len(), 5 + 200);
        for ix in &ixs {
            assert!(
                solana::transaction_size(&[2; 32], std::slice::from_ref(ix))
                    <= solana::PACKET_DATA_SIZE
            );
        }
    }

    #[test]
    fn seeds_are_bounded() {
        assert!(seed_bytes("").is_err());
        assert!(seed_bytes("this-is-17-bytes!").is_err());
        assert_eq!(&seed_bytes("idl").unwrap()[..4], b"idl\0");
    }
}
