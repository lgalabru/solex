# solex — SOlana EXpress

Ship one Solana program to several release channels.

Each channel in `Solex.yaml` is a separate deployment with its own program id,
cluster and upgrade authority. Mark instructions, accounts or whole modules with
`#[channel(stable)]`, `#[channel(nightly)]` or `#[channel([nightly, 850])]`,
and `solex build --channel nightly` compiles exactly what that channel ships.
(The attribute is `channel`, not `feature`: the latter collides with rustc's
built-in `#![feature]`.)
Nothing else makes it into the binary.

> **Status:** early. The macro and CLI surface described here works end to end
> (Anchor 1.0, `cargo build-sbf`), but expect changes.

## Quick start

```sh
cargo install --path crates/cli
solex init counter --cluster stable=mainnet-beta --cluster nightly=devnet
cd counter
solex features                # which items ship on which channel
solex build --channel nightly # cargo build-sbf with the channel selected
solex deploy --channel nightly
```

`init` creates an Anchor program, one keypair per channel under `keys/`
(gitignored), and this manifest:

```yaml
name: counter
default_channel: stable
channels:
  stable:
    program_id: 2mwKUED1kXW5nGmpTrLGbdMBe9miMkstQjUJm9vEUsqH
    keypair: keys/stable.json
    cluster: mainnet-beta
  nightly:
    program_id: 9rwsA9m17bLqtT1GZcjGZiWfzm4yXsu1p6w7AiAPKGMs
    keypair: keys/nightly.json
    cluster: devnet
    includes: [stable]        # nightly ships every #[channel(stable)] item too
```

## In the program

```rust
use anchor_lang::prelude::*;

solex::declare_id!();          // program id of the channel being built

#[solex::program]
pub mod counter {
    use super::*;

    #[channel(stable)]         // on stable, and on nightly via `includes`
    pub fn increment(ctx: Context<Update>) -> Result<()> { /* ... */ }

    #[channel([nightly, 1000])] // nightly only, and refuses to run before epoch 1000
    pub fn reset(ctx: Context<Update>) -> Result<()> { /* ... */ }
}

// Outside a #[solex::program] module, spell the attribute with its path:
#[solex::channel(nightly)]
#[derive(Accounts)]
pub struct Reset<'info> { /* ... */ }
```

### `#[channel(...)]` grammar

| Form | Meaning |
|---|---|
| `#[channel(stable)]` | compiled into channels that ship the `stable` tag |
| `#[channel(stable, nightly)]` | compiled if the channel ships *any* listed tag |
| `#[channel([nightly, 850])]` | compiled into `nightly`; the function returns `FEATURE_NOT_ACTIVE` until epoch 850 |
| `#[channel(nightly = 850)]` | same as above, flatter spelling |
| `#[channel(stable, [nightly, 850])]` | unconditional on `stable`, epoch-gated on a nightly-only build |

A channel ships its own tag plus the tags of every channel it `includes`,
transitively. An item whose tags the channel does not ship is removed from the
compilation entirely, so it has no discriminator, no IDL entry and no code.

Epoch bounds are runtime gates: the macro injects a Clock sysvar check at the
top of the handler, which fails with `ProgramError::Custom(0x53580000)`
(`solex::error::FEATURE_NOT_ACTIVE`) before the activation epoch. They are only
valid on functions.

Other macros: `solex::active_channel!()` is the active channel name as a
`&'static str`; `solex::ships!(nightly)` is a `bool` literal saying whether the
active channel ships that tag; `solex::current_epoch()`
and `solex::epoch_reached(n)` are available for manual checks.

## CLI

| Command | What it does |
|---|---|
| `solex init <name>` | scaffold a project; `--channels a,b,c`, `--cluster ch=cluster` |
| `solex features [--channel c] [--json] [--check]` | static matrix of every `#[channel]` item per channel; `--check` fails on tags no channel defines |
| `solex channels [--json]` | channels, program ids, clusters, resolved tag sets |
| `solex build [--channel c \| --all] [--dry-run]` | `cargo build-sbf` into `target/solex/<channel>/` |
| `solex deploy [--channel c] [--build] [--dry-run]` | `solana program deploy` with the channel's id, cluster and authority |
| `solex keys list \| new --channel c \| check` | per-channel keypairs; `check` verifies keypairs match `program_id` |
| `solex test [--channel c \| --all] [-- args]` | `cargo test` with the channel selected |
| `solex channel` | print the resolved channel (for scripts) |

Channel selection everywhere: `--channel`, else `SOLEX_CHANNEL`, else
`default_channel`, else the only channel defined.

## How the channel reaches the compiler

The proc macros read `Solex.yaml` (found by walking up from
`CARGO_MANIFEST_DIR`, or at `SOLEX_MANIFEST`) and `SOLEX_CHANNEL` at expansion
time. The scaffolded `build.rs` declares both as `rerun-if-env-changed`, so
switching channels re-expands the program crate without rebuilding its
dependencies, and each channel gets its own output directory.

## Frameworks

`solex` defaults to the `anchor` feature: `#[solex::program]` applies channel
gating and then hands the module to `#[anchor_lang::program]`, and
`declare_id!` wraps Anchor's. With `default-features = false, features =
["native"]` the same macros work on framework-free programs built on the
`solana-*` crates. Pinocchio support is planned.

## Repository layout

| Crate | Purpose |
|---|---|
| `crates/manifest` | `Solex.yaml` model, channel/tag resolution, `#[channel]` grammar |
| `crates/macros` | proc macros (`channel`, `program`, `declare_id`, `active_channel`, `ships`) |
| `crates/solex` | the crate programs depend on: macros re-exported + epoch gate runtime |
| `crates/core` | build / deploy / scan / scaffold logic, no terminal output |
| `crates/cli` | the `solex` binary, one module per command |
| `tests/native` | macro integration tests on a framework-free program, run once per channel |
| `examples/counter` | Anchor program produced by `solex init` |

```sh
just test            # workspace tests + native tests on all three fixture channels
just example-check   # host type-check of the Anchor example per channel
just example-build   # cargo build-sbf of the example for every channel
```

## License

MIT.
