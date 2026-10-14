# Hand-rolled dispatch (Pinocchio and native programs)

Pinocchio programs decode the instruction byte themselves. A feature spans
the whole public surface of an instruction family; gate each piece with the
same Cargo feature:

```rust
#![no_std]
solex::declare_id!();                     // channel's program id

pub const TREASURY_OWNER: [u8; 32] =
    const_crypto::bs58::decode_pubkey(solex::var!("treasury_owner"));

#[repr(u8)]
pub enum Instruction<'a> {
    Open(OpenArgs<'a>) = 1,
    #[cfg(feature = "hooks")]
    OpenWithHooks(OpenArgs<'a>) = 10,
}

impl<'a> Instruction<'a> {
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, ProgramError> {
        let (disc, rest) = data.split_first().ok_or(ProgramError::InvalidInstructionData)?;
        match *disc {
            open::DISCRIMINATOR => Ok(Self::Open(OpenArgs::load(rest)?)),
            #[cfg(feature = "hooks")]
            open::WITH_HOOKS_DISCRIMINATOR => Ok(Self::OpenWithHooks(OpenArgs::load_with_hooks(rest)?)),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

fn process_instruction(program_id: &Address, accounts: &mut [AccountView], data: &[u8]) -> ProgramResult {
    match Instruction::from_bytes(data)? {
        Instruction::Open(args) => open::process(program_id, accounts, &args),
        #[cfg(feature = "hooks")]
        Instruction::OpenWithHooks(args) => open::process_with_hooks(program_id, accounts, &args),
    }
}

#[cfg(feature = "hooks")]
pub const WITH_HOOKS_DISCRIMINATOR: u8 = 10;

#[cfg(feature = "hooks")]
#[solex::activation("hooks")]             // epoch gate where the manifest declares one
pub fn process_with_hooks(...) -> ProgramResult { ... }
```

```toml
# Cargo.toml
[features]
hooks = []

# Solex.toml
[features.hooks]
channels = ["devnet"]
```

Checklist for one gated instruction family:

- discriminator constant, enum variant, `from_bytes` arm, dispatch arm,
  `name()` arm, handler entry point: all carry `#[cfg(feature = "hooks")]`;
  a missing one is a compile error on the channel that enables the feature;
- shared internals (`process_inner`, account validation, version checks) stay
  unconditional, so stable keeps rejecting accounts it does not understand;
- `solex features` shows the feature as one row with every marker behind it.

Cargo setup:

```toml
solex-lang = { version = "0.2", default-features = false, features = ["pinocchio"] }
```

and a `build.rs` with `cargo:rerun-if-env-changed=SOLEX_CHANNEL`,
`cargo:rerun-if-env-changed=SOLEX_MANIFEST`, `cargo:rerun-if-env-changed=SOLEX_CLUSTER`
and `cargo:rerun-if-changed=<manifest>`.
