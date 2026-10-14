# Manifest

Standalone `Solex.toml` (or `Solex.yaml` with the same keys):

```toml
name = "payment_channels"          # optional, defaults to the Cargo package name
default_channel = "localnet"
program_path = "program/payment_channels"   # optional, defaults to the manifest dir

[build]                                      # optional; default is cargo build-sbf
args = ["--tools-version", "v1.58", "--arch", "v3"]
# For another pipeline, e.g. sbpf-linker:
# command = ["cargo", "+nightly", "build-bpf"]
# artifact = "target/bpfel-unknown-none/release/lib{lib}.so"

[channels.mainnet]
program_id = "CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX"
cluster = "mainnet-beta"
upgrade_authority = "~/.config/solana/mainnet-authority.json"
vars = { treasury_owner = "Cs2zdfUNonRdRGsiZUQQLdTxzxVvJZmgiX2mpLYKuEqP" }

[channels.devnet]
includes = ["mainnet"]
program_id = "CHNLxBtXw7VPqd5GMcbtVoEbn4ehyaSoiHj56Ao5sWkJ"
cluster = "devnet"
keypair = "keys/devnet.json"
vars = { treasury_owner = "4zTeC5mVqWLruDexgU2mV66p9t5vCA9JyiZqdGDUspap" }

[channels.localnet]
includes = ["devnet"]
program_id = "CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX"
cluster = "localnet"
vars = { treasury_owner = "DoWeHaveAPlaceholder1111111111111111111111" }
```

```toml
[features.hooks]
description = "V2 lifecycle hooks"
channels = ["devnet"]          # enabled on devnet and on channels that include it
epoch = { mainnet = 850 }      # #[solex::activation("hooks")] handlers gate until then
```

Every feature here must also exist in the program's `Cargo.toml` `[features]`
(usually `hooks = []`). `solex build`, `solex test` and `solex env` enable the
channel's features; code gates on them with `#[cfg(feature = "hooks")]`.

Fields per channel: `program_id` (required to build; a channel without one is
tag-only and exists for others to `include`), `cluster` (moniker or RPC URL,
or a list), `keypair` (first deploy only), `upgrade_authority`, `includes`,
`vars`, `description`.

Per-cluster overrides, `[channels.<channel>.<cluster>]`, may set `program_id`,
`keypair`, `upgrade_authority` and `vars` (merged over the channel's). The
key must be one of the channel's clusters. With overrides, the channel builds
once per cluster and `SOLEX_CLUSTER` reaches the macros; `solex::active_cluster!()`
exposes it.

## Embedded in a host manifest

The same section under a `solex` key in `Anchor.toml` or `Quasar.toml`:

```toml
[solex]
default_channel = "stable"
program_path = "programs/counter"

[solex.channels.stable]
program_id = "..."
cluster = "mainnet-beta"
```

Discovery walks up from the crate being built: `Solex.toml`, `Solex.yaml`,
then `Anchor.toml` / `Quasar.toml` containing a `solex` section.
`SOLEX_MANIFEST=<path>` overrides it.

## Tag resolution

A channel ships its own name plus every channel it `includes`, transitively.
`#[channel(a, b)]` ships when the channel ships `a` or `b`. With the manifest
above, `#[channel(devnet)]` is in `devnet` and `localnet`, not `mainnet`.
