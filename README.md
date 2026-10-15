<div align="center">
  <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/lgalabru/solex/main/docs/assets/solex-github-hero-dark.png">
      <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/lgalabru/solex/main/docs/assets/solex-github-hero-light.png">
      <img alt="SolEx. Every increment ships. Only the proven ones reach stable." style="max-width: 60%;">
  </picture>
</div>

## 🚆 Overview

Solex (SOlana EXpress) ships one Solana program to several release channels. A channel is a deployment: its own program id, cluster and upgrade authority, declared once in `Solex.toml`.

A feature is a Cargo feature of the program whose enablement solex decides per channel. Declare it once in the manifest with the channels that ship it, gate code with plain `#[cfg(feature = "...")]`, and `solex build --canary` compiles exactly what canary ships. Promoting a feature to stable is a one-line manifest edit; the source does not change.

Agents produce code faster than anyone can review a big release. Solex keeps every upgrade small enough to test, review and audit as one diff, and the stable artifact carries nothing dormant.

> **Status:** 0.2.1 on crates.io. Anchor 1.0 and Pinocchio programs build end to end with `cargo build-sbf`; see [solana-foundation/payment-channels#96](https://github.com/solana-foundation/payment-channels/pull/96) for a real program on two channels. Expect the API to move before 1.0.

## 💡 Key Features

### 📡 Channels in one manifest

`stable`, `canary`, `canary`, or whatever your team calls them. Each channel lists its program id, cluster(s), keypair and the channels it includes. Write it as `Solex.toml`, `Solex.yaml`, or a `[solex]` section inside `Anchor.toml` or `Quasar.toml`.

### ✂️ Features are Cargo features, enabled per channel

`[features.hooks] channels = ["canary"]` in the manifest, `hooks = []` in `Cargo.toml`, `#[cfg(feature = "hooks")]` in the code. `solex build --stable` passes no `--features hooks`, so the stable binary has no dormant hooks: no discriminator, no IDL entry, no bytes.

### ⏱️ Epoch gates

`[features.hooks] epoch = { stable = 850 }` plus `#[solex::activation("hooks")]` on the handler: compiled in, but `FEATURE_NOT_ACTIVE` until the cluster reaches epoch 850, the way Solana's own feature gates activate.

### 🔍 What ships where, before you build

`solex features` reads the manifest and the sources and prints one entry per feature with the channels it ships on. Reviewers and auditors see the same view the compiler acts on, and `--check` fails on a feature missing from `Cargo.toml`, declared but unused, or a build script that would let cargo reuse another channel's compilation.

### 🔑 Per-deployment constants

Treasury owners, admin keys and fee accounts live next to the program id as `vars` on each channel and are read with `solex::var!("treasury_owner")`. A channel without the var does not compile, so a placeholder can never reach a real cluster.

### 🚀 Build, deploy, terminate per channel

`solex build`, `solex deploy` and `solex terminate` take the channel's program id, cluster, authority and features from the manifest. Other pipelines get the same selection through `solex env` and `solex exec`.

## Installation

**From crates.io**

```console
cargo install solex
```

**From source**

```console
git clone https://github.com/lgalabru/solex
cd solex
just install        # cargo install --path crates/cli --force
```

Requires the Solana CLI with `cargo build-sbf` for program builds. Programs depend on `solex-lang`; `solex setup` installs the agent skill and registers the MCP server with Claude Code and Codex.

## Usage

Scaffold an Anchor program with two channels and a keypair for each:

```console
solex init counter --cluster stable=mainnet --cluster canary=devnet
cd counter
```

See the channels and what each one ships:

```console
$ solex channels
  ┌────────────────────────────────────────────────────────────────────────────┐
  │ stable  mainnet                                               default │
  │ program   2mwK…UsqH                                                        │
  ├────────────────────────────────────────────────────────────────────────────┤
  │ canary  devnet                                                             │
  │ program   9rws…KGMs                                                        │
  │ includes  stable                                                           │
  │ features  reset (from epoch 1000)                                          │
  └────────────────────────────────────────────────────────────────────────────┘

$ solex features
  ┌────────────────────────────────────────────────────────────────────────────┐
  │ #[channel(stable)]  stable  canary                                 2 gates │
  ├────────────────────────────────────────────────────────────────────────────┤
  │ reset  canary from epoch 1000                                       1 gate │
  │ resets the counter; canary-only until proven                               │
  └────────────────────────────────────────────────────────────────────────────┘
```

Build and deploy a channel. Any declared channel name works as a flag:

```console
$ solex build --canary
  ┌────────────────────────────────────────────────────────────────────────────┐
  │ canary  devnet                                             98.9 KiB  761ms │
  │ program   9rws…KGMs                                                        │
  │ features  reset                                                            │
  │ artifact  target/solex/canary/counter.so                                   │
  └────────────────────────────────────────────────────────────────────────────┘
$ solex deploy --canary
✔ Deploy canary to (space toggles, enter confirms) · devnet  9rws…KGMs
  ┌────────────────────────────────────────────────────────────────────────────┐
  │ canary  devnet                                                             │
  │ program   9rws…KGMs                                                        │
  │ features  reset                                                            │
  │ artifact  target/solex/canary/counter.so                                   │
  └────────────────────────────────────────────────────────────────────────────┘
│ Deployed canary to devnet
│ 9rwsA9m17bLqtT1GZcjGZiWfzm4yXsu1p6w7AiAPKGMs ↗  [⌘ + click] to open Explorer
```

`deploy` asks which of the channel's clusters to send to; `-y` skips the question in CI.

| Command | Purpose |
|---|---|
| `solex init <name>` | scaffold a project; `--channels a,b,c`, `--cluster ch=cluster`, `--format toml` |
| `solex channels` | deployments, program ids, clusters, includes, features, vars |
| `solex features [name] [--items] [--json] [--check]` | one entry per feature; `name` lists that feature's markers with file:line |
| `solex build [--<channel> \| --all] [--cluster c]` | `cargo build-sbf` with the channel's features into `target/solex/<channel>/` |
| `solex deploy [--<channel>] [--build] [--dry-run] [-y] [-- <args>]` | `solana program deploy` with the channel's id, cluster and authority; asks which cluster(s), `-y` skips; `-- <args>` go to the Solana CLI |
| `solex terminate [--<channel>] [--buffers] --yes` | `solana program close`; reclaims rent; mainnet needs `--allow-mainnet` |
| `solex test [--<channel>] [-- args]` | `cargo test` with the channel selected and its features enabled |
| `solex env [--format shell\|dotenv\|json]` | `SOLEX_CHANNEL`, `SOLEX_MANIFEST`, `SOLEX_CLUSTER`, `SOLEX_FEATURES` for other pipelines |
| `solex exec [--<channel>] -- <cmd>` | run any command with the channel selected |
| `solex keys list \| new [--cluster x] \| check` | per-channel program keypairs |
| `solex setup` | install the agent skill and register the MCP server with Claude Code and Codex; `--no-skill`, `--no-mcp` |
| `solex mcp` | MCP server: channels, features, builds, deploy plans |

Channel selection everywhere: `--<channel>` or `--channel <name>`, else `SOLEX_CHANNEL`, else `default_channel` in the manifest.

## In the program

```toml
# Cargo.toml
[features]
hooks = []
```

```toml
# Solex.toml
default_channel = "stable"

[channels.stable]
description = "V1 instruction set, the audited surface"
program_id = "CHNLxYvV…"
cluster = "mainnet"
vars = { treasury_owner = "Cs2zdf…" }

[channels.canary]
description = "stable plus the hooks feature"
includes = ["stable"]
program_id = "CHNLxBtX…"
cluster = "devnet"
vars = { treasury_owner = "4zTeC5…" }

[features.hooks]
description = "V2 lifecycle hooks"
channels = ["canary"]             # add "stable" here to promote
epoch = { stable = 850 }           # once on stable, inactive until epoch 850
```

```rust
solex::declare_id!();              // program id of the channel being built

pub const TREASURY_OWNER: [u8; 32] = decode_pubkey(solex::var!("treasury_owner"));

#[cfg(feature = "hooks")]
OpenWithHooks(OpenArgs<'a>) = 10,  // any item, variant, arm or field: plain cfg

#[cfg(feature = "hooks")]
#[solex::activation("hooks")]      // runtime epoch gate where the manifest declares one
pub fn open_with_hooks(...) -> ProgramResult { ... }
```

```console
$ solex features
  ┌────────────────────────────────────────────────────────────────────────────┐
  │ hooks  canary                                                     24 gates │
  │ V2 lifecycle hooks                                                         │
  └────────────────────────────────────────────────────────────────────────────┘
```

Features are declared once, so there is nothing to forget across the 24 markers of a real instruction family. `solex features --check` also catches a feature in the manifest that `Cargo.toml` does not declare, one that no `cfg` uses, and a `build.rs` that does not list `SOLEX_CHANNEL`, `SOLEX_MANIFEST` and `SOLEX_CLUSTER` under `rerun-if-env-changed`, which would let cargo reuse another channel's compilation.

### Channel tags

For code tied to a release train rather than a feature, `#[channel(...)]` gates directly on channel names, with the same `includes` resolution:

| Attribute | Meaning |
|---|---|
| `#[channel(canary)]` | compiled into channels that ship `canary` |
| `#[channel(stable, canary)]` | compiled if the channel ships any listed tag |
| `#[channel([canary, 850])]` | compiled into `canary`; inactive until epoch 850 |

Attribute macros cannot sit on enum variants or `match` arms, so for those `#[solex::gated]` on the enclosing item resolves nested `#[channel]` markers; `#[solex::program]` does this for Anchor modules. Cargo features need none of this.

### One channel, several clusters

`cluster` takes a list. Identical bytes go to every cluster unless a cluster overrides something, written as `[channels.<channel>.<cluster>]`:

```toml
[channels.stable]
program_id = "CHNLxYvV…"
cluster = ["mainnet", "devnet"]
vars = { treasury_owner = "Cs2zdf…" }

[channels.stable.devnet]               # inherits everything else
program_id = "CHNLxBtX…"
vars = { treasury_owner = "4zTeC5…" }
```

Because `var!` and `declare_id!` resolve at compile time, a channel with overrides is one artifact per cluster, `target/solex/stable/mainnet/` and `target/solex/stable/devnet/`, each built with `SOLEX_CLUSTER` set; `solex deploy` sends each to its cluster and `--cluster` narrows any command to one. A build that cannot tell which cluster it is for fails rather than guessing. A channel without `program_id` is tag-only: it names a surface for other channels to `include` and is never built.

## Build pipelines

Solex owns two things at build time: the environment the proc macros read (`SOLEX_CHANNEL`, `SOLEX_MANIFEST`, `SOLEX_CLUSTER`) and the `--features` list. `solex build` is a convenience over that:

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
eval "$(solex env --canary)"            # SOLEX_CHANNEL, SOLEX_MANIFEST, SOLEX_FEATURES=hooks
cargo +nightly build-bpf --features "$SOLEX_FEATURES"
solex exec --canary -- cargo test --features "$SOLEX_FEATURES"
```

Verified builds work the same way: the container running `solana-verify` must see the same variables and features.

## Frameworks

`solex-lang` defaults to Anchor: `#[solex::program]` applies channel gating, then hands the module to `#[anchor_lang::program]`, and `declare_id!` wraps Anchor's. With `default-features = false` and `features = ["pinocchio"]` the crate is `no_std`, `declare_id!` wraps `pinocchio::address::declare_id!`, and the epoch gate reads pinocchio's Clock sysvar. `features = ["native"]` targets the modular `solana-*` crates.

### For framework authors

The crates are split so a framework CLI can offer channels without shelling out to `solex`:

- **`solex-types`** is plain serde data. Deserialize the `solex` section of your own manifest into `solex_types::Channels` and you have channel resolution, `includes`, features per channel and activation epochs. The `yaml`, `toml` and `spec` features are optional.
- **`solex-core`** takes that section through `Project::from_parts` and exposes `build::target_env` and `build::channel_features`, the environment and `--features` your own `cargo build-sbf` call needs, plus the scan, deploy and keypair logic.
- The proc macros find the configuration by walking up from `CARGO_MANIFEST_DIR`, or read the file named by `SOLEX_MANIFEST`. A host manifest with a `solex` key is accepted as-is, so your users keep one config file.

## Architecture & How to Contribute

| Crate | Purpose |
|---|---|
| `crates/types` | `solex-types`: the `solex` manifest section (channels, features, overrides), resolution rules, `#[channel]` grammar; YAML and TOML |
| `crates/macros` | proc macros: `declare_id!`, `var!`, `activation`, `channel`, `gated`, `program` |
| `crates/lang` | `solex-lang`, the crate programs depend on (`use solex::...`); Anchor, Pinocchio or native |
| `crates/core` | `solex-core`: build, deploy, terminate, scan and scaffold logic, reusable from other CLIs |
| `crates/cli` | the `solex` binary (`cargo install solex`) and its display components |
| `crates/mcp` | `solex-mcp`: thin MCP server over core (`solex mcp`) |
| `tests/native` | macro and feature tests on a framework-free program, run once per channel |
| `tests/pinocchio` | `no_std` compile check of the `pinocchio` feature with hand-rolled dispatch |
| `skills/solex` | agent skill (skills.sh compatible) on channels and gating; `solex setup` installs it |
| `examples/counter` | the program `solex init` produces |

```console
just test            # workspace tests, native tests per channel, pinocchio check
just example-check   # type-check the Anchor example per channel
just example-build   # cargo build-sbf of the example for every channel
```

Issues and pull requests are welcome at [github.com/lgalabru/solex](https://github.com/lgalabru/solex/issues).

## License

MIT.
