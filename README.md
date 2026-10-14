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

### 📡 Channels in one YAML file

`stable`, `nightly`, `canary`, or whatever your team calls them. Each channel lists its program id, cluster and keypair, and which other channels it includes.

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
| `solex init <name>` | scaffold a project; `--channels a,b,c`, `--cluster ch=cluster` |
| `solex features [--json] [--check]` | item × channel matrix; `--check` fails on tags no channel defines |
| `solex channels` | channels, program ids, clusters, resolved tag sets |
| `solex build [--channel c \| --all]` | `cargo build-sbf` into `target/solex/<channel>/` |
| `solex deploy [--channel c] [--build]` | `solana program deploy` with the channel's id, cluster and authority |
| `solex keys list \| new \| check` | per-channel program keypairs |
| `solex test [--channel c \| --all]` | `cargo test` with the channel selected |

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

| Attribute | Meaning |
|---|---|
| `#[channel(stable)]` | compiled into channels that ship `stable` |
| `#[channel(stable, nightly)]` | compiled if the channel ships any listed tag |
| `#[channel([nightly, 850])]` | compiled into `nightly`; inactive until epoch 850 |
| `#[channel(stable, [nightly, 850])]` | unconditional on stable, epoch-gated on a nightly-only build |

The attribute works on functions, structs, impls and modules. Epoch gates apply to functions only. `solex::active_channel!()` and `solex::ships!(nightly)` expose the channel to your own code.

## Frameworks

`solex` defaults to Anchor: `#[solex::program]` applies channel gating, then hands the module to `#[anchor_lang::program]`. With `default-features = false, features = ["native"]` the same macros work on framework-free programs. Pinocchio support is planned.

## Architecture & How to Contribute

| Crate | Purpose |
|---|---|
| `crates/manifest` | `Solex.yaml` model, channel resolution, `#[channel]` grammar |
| `crates/macros` | proc macros |
| `crates/lang` | `solex-lang`, the crate programs depend on (`use solex::...`) |
| `crates/core` | build, deploy, scan and scaffold logic |
| `crates/cli` | the `solex` binary (`cargo install solex`) |
| `tests/native` | macro tests on a framework-free program, run once per channel |
| `examples/counter` | the program `solex init` produces |

```console
just test            # workspace tests + native tests on every fixture channel
just example-check   # type-check the Anchor example per channel
just example-build   # cargo build-sbf of the example for every channel
```

Issues and pull requests are welcome at [github.com/lgalabru/solex](https://github.com/lgalabru/solex/issues).

## License

MIT.
