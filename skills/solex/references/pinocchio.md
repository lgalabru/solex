# Hand-rolled dispatch (Pinocchio and native programs)

Pinocchio programs decode the instruction byte themselves. Gate the whole
public surface of a new instruction family:

```rust
#![no_std]
solex::declare_id!();                     // channel's program id

pub const TREASURY_OWNER: [u8; 32] =
    const_crypto::bs58::decode_pubkey(solex::var!("treasury_owner"));

#[solex::gated]
#[repr(u8)]
pub enum Instruction<'a> {
    Open(OpenArgs<'a>) = 1,
    #[channel(devnet, "hooks")]
    OpenWithHooks(OpenArgs<'a>) = 10,
}

#[solex::gated]
impl<'a> Instruction<'a> {
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, ProgramError> {
        let (disc, rest) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
        match *disc {
            open::DISCRIMINATOR => Ok(Self::Open(OpenArgs::load(rest)?)),
            #[channel(devnet, "hooks")]
            open::WITH_HOOKS_DISCRIMINATOR => Ok(Self::OpenWithHooks(OpenArgs::load_with_hooks(rest)?)),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

#[solex::gated]
fn process_instruction(program_id: &Address, accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    match Instruction::from_bytes(data)? {
        Instruction::Open(args) => open::process(program_id, accounts, &args),
        #[channel(devnet, "hooks")]
        Instruction::OpenWithHooks(args) => open::process_with_hooks(program_id, accounts, &args),
    }
}

#[solex::channel(devnet, "hooks")]
pub const WITH_HOOKS_DISCRIMINATOR: u8 = 10;

#[solex::channel(devnet, "hooks")]
pub fn process_with_hooks(...) -> ProgramResult { ... }
```

Checklist for one gated instruction family:

- discriminator constant, enum variant, `from_bytes` arm, dispatch arm,
  `name()` arm, handler entry point: all carry the same tag;
- shared internals (`process_inner`, account validation, version checks) stay
  untagged, so stable keeps rejecting accounts it does not understand;
- `solex features` lists every marker, so a missing arm shows up as a
  compile error on the gated channel and as a missing row in the matrix.

Cargo setup:

```toml
solex-lang = { version = "0.1", default-features = false, features = ["pinocchio"] }
```

and a `build.rs` with `cargo:rerun-if-env-changed=SOLEX_CHANNEL`,
`cargo:rerun-if-env-changed=SOLEX_MANIFEST` and `cargo:rerun-if-changed=<manifest>`.
