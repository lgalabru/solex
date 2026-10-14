<div align="center">
  <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/lgalabru/solex/main/docs/assets/solex-github-hero-dark.png">
      <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/lgalabru/solex/main/docs/assets/solex-github-hero-light.png">
      <img alt="SolEx. Every increment ships. Only the proven ones reach stable." style="max-width: 60%;">
  </picture>
</div>

## 🚆 Overview

Solex (SOlana EXpress) ships one Solana program to several release channels. Each channel in `Solex.yaml` is its own deployment, with its own program id, cluster and upgrade authority.

Tag a change with `#[channel(nightly)]` and it compiles into the nightly binary only. Promote it to `#[channel(stable)]` once devnet has run it. Code that is not tagged for a channel is absent from that channel's artifact, so what mainnet runs is exactly what you chose to ship.

Agents produce code faster than anyone can review a big release. Solex keeps every upgrade small enough to test, review and audit as one diff.

> **Status:** early. Anchor 1.0 and `cargo build-sbf` work end to end; expect the API to move.

## 💡 Key Features

### 📡 Channels in one config section

`stable`, `nightly`, `canary`, or whatever your team calls them. Each channel lists its program id, cluster and keypair, and which other channels it includes. Write it as `Solex.toml`, `Solex.yaml`, or a `[solex]` section inside `Anchor.toml` or `Quasar.toml`.

### ✂️ Gated code is removed at compile time

A nightly instruction has no discriminator, no IDL entry and no bytes in the stable binary. There is no dormant code path on mainnet to audit or exploit.

### ⏱️ Epoch gates

`#[channel([nightly, 850])]` compiles into nightly and returns `FEATURE_NOT_ACTIVE` until the cluster reaches epoch 850, the way Solana's own feature gates activate.

### 🔍 What ships where, before you build

`solex features` reads the sources and prints the item × channel matrix. Reviewers and auditors see the same table the compiler acts on.

### 🚀 Build and deploy per channel

`solex build --channel nightly` and `solex deploy --channel nightly` select the channel, the program id, the cluster and the authority from the manifest. Artifacts land in `target/solex/<channel>/`.

## Installation

```console
cargo install solex
```

Requires the Solana CLI with `cargo build-sbf` for program builds. To build from source, clone the repo and run `cargo install --path crates/cli`.

## Usage

Scaffold an Anchor program with two channels and a keypair for each:

```console
solex init counter --cluster stable=mainnet-beta --cluster nightly=devnet
cd counter
```

See what each channel ships:

```console
$ solex features

  item           #[channel]       stable   nightly
  ─────────────  ───────────────  ──────── ────────
  fn initialize  stable              ✓        ✓       src/lib.rs:12
  fn increment   stable              ✓        ✓       src/lib.rs:19
  fn reset       [nightly, 1000]     –      ✓@1000    src/lib.rs:27
```

Build and deploy a channel:

```console
solex build --channel nightly
solex deploy --channel nightly
```

| Command | Purpose |
|---|---|
| `solex init <name>` | scaffold a project; `--channels a,b,c`, `--cluster ch=cluster`, `--format toml` |
| `solex features [--json] [--check]` | item × channel matrix; `--check` fails on tags no channel defines |
| `solex channels` | channels, program ids, clusters, resolved tag sets |
| `solex build [--channel c \| --all]` | `cargo build-sbf` into `target/solex/<channel>/` |
| `solex deploy [--channel c] [--build]` | `solana program deploy` with the channel's id, cluster and authority |
| `solex terminate [--channel c] [--buffers] --yes` | `solana program close`: reclaim ProgramData or buffer rent; mainnet needs `--allow-mainnet` |
| `solex env [--format shell\|dotenv\|json]` | the channel environment for other build pipelines |
| `solex exec [--channel c] -- <cmd>` | run any command with the channel selected |
| `solex keys list \| new \| check` | per-channel program keypairs |
| `solex test [--channel c \| --all]` | `cargo test` with the channel selected |
| `solex setup [--mcp]` | install the agent skill; register the MCP server with Claude Code |
| `solex mcp` | MCP server: channels, feature matrix, builds, deploy plans |

Channel selection everywhere: `--channel`, else `SOLEX_CHANNEL`, else `default_channel` in the manifest.

## In the program

```rust
use anchor_lang::prelude::*;

solex::declare_id!();          // program id of the channel being built

#[solex::program]
pub mod counter {
    use super::*;

    #[channel(stable)]         // stable, and nightly via `includes`
    pub fn increment(ctx: Context<Update>) -> Result<()> { /* ... */ }

    #[channel([nightly, 1000])] // nightly only, inactive before epoch 1000
    pub fn reset(ctx: Context<Update>) -> Result<()> { /* ... */ }
}
```

```yaml
# Solex.yaml
name: counter
default_channel: stable
channels:
  stable:
    program_id: 2mwKUED1kXW5nGmpTrLGbdMBe9miMkstQjUJm9vEUsqH
    cluster: mainnet-beta
  nightly:
    program_id: 9rwsA9m17bLqtT1GZcjGZiWfzm4yXsu1p6w7AiAPKGMs
    cluster: devnet
    includes: [stable]        # nightly also ships every #[channel(stable)] item
```

The same section can live in a host manifest instead of its own file. Solex looks for `Solex.toml` / `Solex.yaml` first, then for a `solex` key in `Anchor.toml` or `Quasar.toml`, walking up from the crate being built:

```toml
# Anchor.toml
[solex]
default_channel = "stable"
program_path = "programs/counter"

[solex.channels.stable]
program_id = "2mwKUED1kXW5nGmpTrLGbdMBe9miMkstQjUJm9vEUsqH"
cluster = "mainnet-beta"

[solex.channels.nightly]
program_id = "9rwsA9m17bLqtT1GZcjGZiWfzm4yXsu1p6w7AiAPKGMs"
cluster = "devnet"
includes = ["stable"]
```

| Attribute | Meaning |
|---|---|
| `#[channel(stable)]` | compiled into channels that ship `stable` |
| `#[channel(stable, nightly)]` | compiled if the channel ships any listed tag |
| `#[channel([nightly, 850])]` | compiled into `nightly`; inactive until epoch 850 |
| `#[channel(stable, [nightly, 850])]` | unconditional on stable, epoch-gated on a nightly-only build |

The attribute works on functions, structs, impls and modules. Epoch gates apply to functions only. `solex::active_channel!()` and `solex::ships!(nightly)` expose the channel to your own code.

### Hand-rolled dispatch: `#[solex::gated]`

Attribute macros cannot sit on enum variants or `match` arms, which is where Pinocchio-style programs decide what an instruction byte means. Put `#[solex::gated]` on the enclosing item and the markers inside it resolve to `#[cfg]`:

```rust
#[solex::gated]
#[repr(u8)]
pub enum Instruction {
    Open = 1,
    #[channel(nightly)]
    OpenWithHooks = 10,
}

#[solex::gated]
fn process(data: &[u8]) -> ProgramResult {
    match data[0] {
        1 => open(),
        #[channel(nightly)]
        10 => open_with_hooks(),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
```

Variants, fields, arms and statements take a plain tag; epoch bounds stay on functions. `#[solex::program]` applies the same pass to everything inside the module.

### Per-channel constants: `solex::var!`

Values that differ per deployment live next to the program id instead of in mutually exclusive cargo features:

```toml
[channels.mainnet]
program_id = "..."
vars = { treasury_owner = "Cs2zdfUNonRdRGsiZUQQLdTxzxVvJZmgiX2mpLYKuEqP" }
```

```rust
const TREASURY_OWNER: [u8; 32] = decode_pubkey(solex::var!("treasury_owner"));
```

A channel without the var fails to compile, so a placeholder can never reach a real cluster.

### One surface, several deployments

`cluster` takes a list when the same artifact goes to several clusters unchanged. When deployments differ in `program_id` or `vars`, they are separate channels; a channel without `program_id` is a tag-only channel that names the shared surface and is never built:

```toml
[channels.stable]                 # tag only: what #[channel(stable)] means

[channels.mainnet]
includes = ["stable"]
program_id = "CHNLxYvV…"
cluster = ["mainnet-beta", "testnet"]   # identical bytes on both
vars = { treasury_owner = "Cs2zdf…" }

[channels.devnet]
includes = ["stable"]
program_id = "CHNLxBtX…"
cluster = "devnet"
vars = { treasury_owner = "4zTeC5…" }
```

`solex deploy --channel mainnet` deploys to both clusters; `--cluster testnet` picks one. `solex build --all` skips tag-only channels.

## Build pipelines

Solex owns one thing at build time: the environment. The proc macros read the channel from `SOLEX_CHANNEL` (and the manifest from `SOLEX_MANIFEST`) at expansion time, so any pipeline that sets those two variables produces channel-gated output. `solex build` is a convenience over that:

```toml
[build]
# default pipeline: cargo build-sbf, with --manifest-path and --sbf-out-dir added
args = ["--tools-version", "v1.58", "--arch", "v3"]
```

```toml
[build]
# upstream rustc + sbpf-linker (github.com/blueshift-gg/sbpf-linker)
command = ["cargo", "+nightly", "build-bpf"]
artifact = "target/bpfel-unknown-none/release/lib{lib}.so"   # copied to target/solex/<channel>/
```

For a pipeline solex should not drive at all, use the environment directly:

```sh
eval "$(solex env --channel nightly)"            # export SOLEX_CHANNEL=... SOLEX_MANIFEST=...
solex env --channel nightly --format dotenv       # for CI, docker --env-file, solana-verify
solex exec --channel nightly -- cargo +nightly build-bpf
```

The scaffolded `build.rs` declares both variables as `rerun-if-env-changed`, so switching channels re-expands the program crate without rebuilding its dependencies.

## Frameworks

`solex-lang` defaults to Anchor: `#[solex::program]` applies channel gating, then hands the module to `#[anchor_lang::program]`. With `default-features = false` and `features = ["pinocchio"]` the crate is `no_std`, `declare_id!` wraps `pinocchio::address::declare_id!`, and the epoch gate reads pinocchio's Clock sysvar. `features = ["native"]` targets the modular `solana-*` crates.

### For framework authors

The crates are split so a framework CLI can offer channels without shelling out to `solex`:

- **`solex-types`** is plain serde data. Deserialize the `solex` section of your own manifest into `solex_types::Channels` and you have tag resolution, `includes`, and `resolve()` for free. Features `yaml`, `toml` and `spec` are all optional.
- **`solex-core`** takes that section through `Project::from_parts` and exposes `build::channel_env` (the environment your `cargo build-sbf` call needs so the proc macros resolve the channel), plus the scan, deploy and keypair logic.
- The proc macros find the configuration by walking up from `CARGO_MANIFEST_DIR`, or read the file named by `SOLEX_MANIFEST`. A host manifest with a `solex` key is accepted as-is, so your users keep one config file.

## Architecture & How to Contribute

| Crate | Purpose |
|---|---|
| `crates/types` | `solex-types`: the embeddable `solex` section, channel resolution, `#[channel]` grammar; YAML and TOML |
| `crates/macros` | proc macros |
| `crates/lang` | `solex-lang`, the crate programs depend on (`use solex::...`) |
| `crates/core` | `solex-core`: build, deploy, scan and scaffold logic, reusable from other CLIs |
| `crates/cli` | the `solex` binary (`cargo install solex`) |
| `tests/native` | macro tests on a framework-free program with a `Solex.toml`, run once per channel |
| `tests/pinocchio` | `no_std` compile check of the `pinocchio` feature with hand-rolled dispatch |
| `crates/mcp` | `solex-mcp`: thin MCP server over core (`solex mcp`) |
| `skills/solex` | agent skill (skills.sh compatible) on channels and gating; `solex setup` installs it |
| `examples/counter` | the program `solex init` produces |

```console
just test            # workspace tests + native tests on every fixture channel
just example-check   # type-check the Anchor example per channel
just example-build   # cargo build-sbf of the example for every channel
```

Issues and pull requests are welcome at [github.com/lgalabru/solex](https://github.com/lgalabru/solex/issues).

## License

MIT.
