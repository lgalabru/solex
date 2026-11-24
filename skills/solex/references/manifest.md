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
cluster = "mainnet"
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
tag-only and exists for others to `include`), `cluster` (`mainnet`, `devnet`,
`testnet`, `localnet` or an RPC URL, or a list; `mainnet-beta` is accepted
too), `keypair` (first deploy only), `upgrade_authority` (a keypair file, the
address of an authority whose key is not on this machine, or
`squads:<multisig account>` for a Squads multisig), `includes`,
`vars`, `description`.

Before uploading, `solex deploy` reads the program's upgrade authority from
the cluster. A declared keypair or address that is not that authority stops
the deploy; with nothing declared, the chain's authority is used. For an
ordinary-key authority with no keypair here, deploy uploads the buffer with
the fee payer and writes the transaction the authority must sign to
`target/solex/<channel>/upgrade.tx`, plus a `.vow` review file when the `vow`
command is installed (devnet and mainnet). The program is not upgraded until
that transaction is signed and sent, and it expires with its blockhash, about
a minute. First deploys to a key held elsewhere and undeclared
program-derived authorities stop before sending anything.

For a Squads multisig, declare the multisig account, never the vault:
`upgrade_authority = "squads:<multisig>"` (`/<n>` picks another vault). Solex
derives the vault and checks the multisig on the cluster at every deploy.
With the vault in control, `solex deploy` uploads a buffer, hands it to the
vault, proposes the upgrade in the multisig, approves it and executes it once
the threshold is reached, as far as the fee payer's key is allowed to.
Running the same deploy again (any member's key) continues the same proposal;
state is in `target/solex/<channel>/upgrade-pending.json`. A non-member stops
after the buffer and prints the buffer and spill addresses. While a
local keypair still holds the program (first deploy, or not handed over yet),
`solex deploy` announces the handover, deploys under that keypair and then
asks a person at a terminal to confirm it; with `-y` or on mainnet it prints
the command instead. `solex authority` shows whether the chain
agrees with the manifest; `solex authority --transfer --vault <address>` hands
a program held by a local keypair to the declared multisig. It is permanent:
never run it for the user, show them the command. `--vault` is the vault
address from the Squads app, which must match the derived one.

Per-cluster overrides, `[channels.<channel>.<cluster>]`, may set `program_id`,
`keypair`, `upgrade_authority` and `vars` (merged over the channel's). The
key must be one of the channel's clusters. With overrides, the channel builds
once per cluster into `target/solex/<channel>/<cluster>/`, `SOLEX_CLUSTER`
reaches the macros and `solex::active_cluster!()` exposes it; `build`,
`test`, `env`, `exec`, `deploy` and `terminate` take `--cluster` to pick one.
Without overrides a multi-cluster channel is a single artifact deployed to
each cluster.

```toml
[channels.stable]
program_id = "CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX"
cluster = ["mainnet", "testnet"]
vars = { treasury_owner = "Cs2zdfUNonRdRGsiZUQQLdTxzxVvJZmgiX2mpLYKuEqP" }

[channels.stable.testnet]
vars = { treasury_owner = "4zTeC5mVqWLruDexgU2mV66p9t5vCA9JyiZqdGDUspap" }
```

## Embedded in a host manifest

The same section under a `solex` key in `Anchor.toml` or `Quasar.toml`:

```toml
[solex]
default_channel = "stable"
program_path = "programs/counter"

[solex.channels.stable]
program_id = "..."
cluster = "mainnet"
```

Discovery walks up from the crate being built: `Solex.toml`, `Solex.yaml`,
then `Anchor.toml` / `Quasar.toml` containing a `solex` section.
`SOLEX_MANIFEST=<path>` overrides it.

## Tag resolution

A channel ships its own name plus every channel it `includes`, transitively.
`#[channel(a, b)]` ships when the channel ships `a` or `b`. With the manifest
above, `#[channel(devnet)]` is in `devnet` and `localnet`, not `mainnet`.
