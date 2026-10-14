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

### ✂️ Features are Cargo features, enabled per channel

Declare `[features.hooks]` once in the manifest with the channels that ship it; gate code with plain `#[cfg(feature = "hooks")]`. `solex build --channel mainnet` passes exactly the features mainnet enables, so the stable binary has no dormant hooks: no discriminator, no IDL entry, no bytes.

### ⏱️ Epoch gates

`[features.hooks] epoch = { mainnet = 850 }` plus `#[solex::activation("hooks")]` on the handler: compiled in, but `FEATURE_NOT_ACTIVE` until the cluster reaches epoch 850, the way Solana's own feature gates activate.

### 🔍 What ships where, before you build

`solex features` reads the manifest and the sources and prints one row per feature with a tick per channel. Reviewers and auditors see the same table the compiler acts on, and `--check` fails on a feature that is declared but unused, unused but declared, or missing from `Cargo.toml`.

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

See what each channel ships, one row per feature:

```console
$ solex features

  counter  1 gated item in 1 file, 2 channels

  feature  channels  gates   stable   nightly
  ───────  ────────  ─────  ───────  ────────
  reset    nightly       1     –      ✓@1000
    resets the counter; nightly-only until proven
    reset
```

`--items` lists every marker with its file and line; `--json` is for tooling.

Build and deploy a channel:

```console
solex build --channel nightly
solex deploy --channel nightly
```

| Command | Purpose |
|---|---|
| `solex init <name>` | scaffold a project; `--channels a,b,c`, `--cluster ch=cluster`, `--format toml` |
| `solex features [--items] [--json] [--check]` | what ships where, one row per gate; `--items` lists every marker; `--check` fails on tags no channel defines |
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

```toml
# Cargo.toml
[features]
hooks = []
```

```toml
# Solex.toml
default_channel = "localnet"

[channels.mainnet]
program_id = "CHNLxYvV…"
cluster = "mainnet-beta"

[channels.devnet]
includes = ["mainnet"]
program_id = "CHNLxBtX…"
cluster = "devnet"

[features.hooks]
description = "V2 lifecycle hooks"
channels = ["devnet"]              # enabled on devnet and on channels that include it
epoch = { mainnet = 850 }          # if it ever ships on mainnet, inactive until epoch 850
```

```rust
solex::declare_id!();              // program id of the channel being built

#[cfg(feature = "hooks")]
OpenWithHooks(OpenArgs<'a>) = 10,  // any item, variant, arm or field: plain cfg

#[cfg(feature = "hooks")]
#[solex::activation("hooks")]      // runtime epoch gate where the manifest declares one
pub fn open_with_hooks(...) -> ProgramResult { ... }
```

Features are declared once. Moving `hooks` to mainnet is editing `channels = [...]`; no source changes, and nothing to forget. `solex build`, `solex test` and `solex env` pass the channel's features as `--features`.

```console
$ solex features

  feature  channels  gates  mainnet  testnet  devnet  localnet
  ───────  ────────  ─────  ───────  ───────  ──────  ────────
  hooks    devnet       24     –        –       ✓        ✓
    V2 lifecycle hooks
    OpenWithHooks, TopUpWithHooks, DistributeWithHooks, WithdrawPayerWithHooks
    process_with_hooks ×3, load_with_hooks
    + 12 match arms, 4 consts
```

### Channel tags

For code tied to a release train rather than a feature, `#[channel(...)]` gates directly on channel names, with the same `includes` resolution:

| Attribute | Meaning |
|---|---|
| `#[channel(nightly)]` | compiled into channels that ship `nightly` |
| `#[channel(stable, nightly)]` | compiled if the channel ships any listed tag |
| `#[channel([nightly, 850])]` | compiled into `nightly`; inactive until epoch 850 |

Attribute macros cannot sit on enum variants or `match` arms, so for those `#[solex::gated]` on the enclosing item resolves nested `#[channel]` markers; `#[solex::program]` does this for Anchor modules. Cargo features need none of this: `#[cfg]` is allowed everywhere.

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

### One channel, several clusters

`cluster` takes a list. Identical bytes go to every cluster unless a cluster overrides something, written as `[channels.<channel>.<cluster>]`:

```toml
[channels.stable]
program_id = "CHNLxYvV…"
cluster = ["mainnet-beta", "devnet"]
vars = { treasury_owner = "Cs2zdf…" }

[channels.stable.devnet]               # inherits everything else
program_id = "CHNLxBtX…"
vars = { treasury_owner = "4zTeC5…" }
```

Because `var!` and `declare_id!` resolve at compile time, a channel with overrides is one artifact per cluster: `target/solex/stable/mainnet-beta/` and `target/solex/stable/devnet/`, each with `SOLEX_CLUSTER` set, and `solex deploy` sends each to its cluster. `--cluster` narrows any command to one. A channel without `program_id` is tag-only: it names a surface for other channels to `include` and is never built.

## Build pipelines

Solex owns the environment and the feature list at build time. The proc macros read `SOLEX_CHANNEL` (and `SOLEX_MANIFEST`, `SOLEX_CLUSTER`) at expansion time; the channel's features go to cargo as `--features`. `solex build` is a convenience over that:

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

For a pipeline solex should not drive at all:

```sh
eval "$(solex env --channel devnet)"     # SOLEX_CHANNEL, SOLEX_MANIFEST, SOLEX_FEATURES=hooks
cargo +nightly build-bpf --features "$SOLEX_FEATURES"
solex exec --channel devnet -- cargo test --features "$SOLEX_FEATURES"
```

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
