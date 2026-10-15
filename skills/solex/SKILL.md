---
name: solex
description: |
  Release channels for Solana programs with solex: ship one codebase as stable / canary deployments, declare Cargo features per channel in Solex.toml and gate code with #[cfg(feature = ...)], epoch-gate activation, keep per-deployment constants in the manifest, and build/deploy per channel.
  TRIGGERS: "feature gate this instruction", "ship to canary first", "release channel", "what ships on mainnet", "solex", "Solex.toml", "Solex.yaml", "#[channel]", "[features.", "promote to stable", "devnet-only instruction", "epoch activation", "deploy per cluster", "deploy to another network", "also deploy on testnet", "channel-gated build"
  Use when adding a feature to a channel, deploying a channel to another cluster, gating, promoting or auditing program features across deployments, or when a program has cfg/feature flags that select clusters or program ids.
---

# solex: release channels for Solana programs

A channel is a deployment: its own program id, cluster and upgrade authority,
declared once in `Solex.toml` / `Solex.yaml` or under `[solex]` in `Anchor.toml`.
A feature is a Cargo feature of the program crate whose enablement solex
decides per channel, declared once under `[features.<name>]` with the
channels that ship it. Code gates on it with plain `#[cfg(feature = "...")]`.
`solex build --channel X` passes exactly X's features, so each artifact
contains one channel's features and nothing dormant.

Why it matters: agents produce more code than a big release can absorb. Small
increments that reach canary first, then stable, are testable and auditable
as one diff each. The stable artifact stays small.

# Core workflow

1. `solex channels` to see the deployments and the features each enables.
   If there is no manifest, `solex init <name>` or add a `[solex]` section.
2. Declare the feature once: `hooks = []` in `Cargo.toml` `[features]`, and in
   the manifest:
   ```toml
   [features.hooks]
   description = "V2 lifecycle hooks"
   channels = ["canary"]
   ```
3. Gate every piece of the feature with `#[cfg(feature = "hooks")]`: enum
   variants, `match` arms, handlers, discriminators, accounts, state. A
   missing marker is a normal compile error.
4. `solex features --check`: one block per feature with the channels that
   ship it and its gate count; errors for a feature missing from
   `Cargo.toml`, declared but unused, a `#[channel]` tag that is not a
   channel, or a `build.rs` that does not watch `SOLEX_*`. Paste the output
   into the PR.
5. `solex build --canary`, `solex test --canary`, `solex deploy --canary`
   (any channel name works as a flag; `--channel canary` is the long form).
6. Promote in the manifest: add `stable` to `channels = [...]`. The source
   does not change.

# Adding a feature to a channel

Two different asks; check which one the user means.

**A new feature** (code that does not exist yet) touches three places, all
required:

| Where | What | Why |
|---|---|---|
| `Cargo.toml` | `hooks = []` under `[features]` | cargo must know the feature; solex passes `--features hooks` |
| manifest | `[features.hooks] channels = ["canary"]` | the only place that says where it ships |
| source | `#[cfg(feature = "hooks")]` on every variant, arm, handler, account | nothing dormant in other channels |

Then `solex features --check` and `solex features hooks` (lists every marker
with file:line) to confirm the gates match the manifest. Add
`#[solex::activation("hooks")]` on handlers only if an `epoch` is declared.

**An existing feature on another channel** is a manifest-only change: add the
channel to `[features.hooks] channels`. Remember `includes`: a feature on
`stable` is also on every channel that includes `stable`, so adding it to a
base channel enables it everywhere above. Rebuild that channel, regenerate its
IDL and clients, and follow the promotion checklist when the target is a
production channel.

Never gate with `#[channel(canary)]` to "add a feature": that repeats the
"where" on every marker and is exactly what the manifest exists to avoid.

# Deploying a channel to another network

First decide whether the user wants the *same code* on another cluster or a
*new release stage*. A new stage (different features) is a new channel that
`includes` the existing one. The same code on another cluster is one channel
with several clusters:

```toml
[channels.stable]
program_id = "CHNL..."                 # same address on every cluster
cluster = ["mainnet-beta", "testnet"]  # was: cluster = "mainnet-beta"
vars = { treasury_owner = "Cs2z..." }

[channels.stable.testnet]              # only if something differs there
vars = { treasury_owner = "4zTe..." }
# program_id, keypair and upgrade_authority can be overridden the same way
```

Steps:

1. Turn `cluster` into a list (monikers or RPC URLs). The program id is an
   address, so the same keypair deploys to every cluster unless the user wants
   a distinct id there; add `[channels.<channel>.<cluster>]` with `program_id`
   and `keypair` in that case.
2. Check every `vars` entry: a treasury, admin or mint that exists on mainnet
   does not exist on testnet. Override the ones that differ under
   `[channels.<channel>.<cluster>]`. Overrides may only name a cluster the
   channel lists; a typo is an error.
3. Without overrides the channel is one artifact and `solex build --channel c`
   is unchanged. With overrides it builds once per cluster into
   `target/solex/<channel>/<cluster>/`, `SOLEX_CLUSTER` reaches the macros and
   `solex::active_cluster!()` exposes it. `build`, `test`, `env` and `exec`
   then take `--cluster <name>`; without it, `build` does every cluster and
   the others fail on ambiguity.
4. Make sure the program's `build.rs` lists
   `cargo:rerun-if-env-changed=SOLEX_CLUSTER` next to `SOLEX_CHANNEL` and
   `SOLEX_MANIFEST`; otherwise cargo reuses the previous cluster's compilation
   and the artifact carries the wrong constants. `solex features --check`
   flags a missing one.
5. `solex channels` shows the resolved values per cluster. Then
   `solex deploy --channel c --cluster testnet --dry-run`, review, deploy.
   Without `--cluster`, `deploy` goes to every cluster of the channel in
   order; `terminate` requires `--cluster` when there are several.
6. A `[solex]` section inside `Anchor.toml` works the same; the `[provider]`
   cluster there is not consulted.

# Rules

- One feature, one declaration. Never repeat "where" in the source; the
  manifest owns it. Prefer a feature over `#[channel(canary)]` tags unless
  code is genuinely tied to a release train.
- Gate the public surface, not shared checks. Validation and version checks
  used by stable paths stay unconditional.
- Activation epochs live in the manifest: `epoch = { stable = 850 }` on the
  feature, plus `#[solex::activation("hooks")]` on the handler. It is a no-op
  on channels without an epoch and returns `FEATURE_NOT_ACTIVE`
  (`ProgramError::Custom(0x53580000)`) before the epoch elsewhere.
- Per-deployment constants (treasury, admin, fee accounts) go under `vars` on
  each channel and are read with `solex::var!("name")`. A channel missing the
  var fails to compile; never keep a placeholder constant in code.
- One channel, several clusters is for the same code on several networks;
  a different feature set is a new channel. See "Deploying a channel to
  another network".
- Cargo features are additive: a workspace build that enables a feature for a
  client or test crate enables it in the program too. Build programs through
  `solex build` (it uses `--manifest-path`) or with `--features
  "$SOLEX_FEATURES"` from `solex env`.

# Before promoting

- Did the feature's channel run it on its cluster with real clients? Link the
  transactions in the PR.
- Does promotion create a new account layout? Once accounts exist, a rollback
  to a binary without that layout strands them. Promote state formats
  deliberately and document the rollback boundary.
- Does the IDL for stable change? Regenerate and diff the IDL per channel.
- Run `solex features --json --channel stable` and confirm only intended rows
  show `ships: true`.

# CLI

- `solex channels [--json]`: deployments, ids, clusters, vars, resolved tags.
- `solex features [name] [--channel c] [--items] [--json] [--check]`: one
  block per feature with the channels that ship it; `solex features hooks`
  lists that feature's markers with file:line; `--items` lists all markers.
- `solex build --channel c [-- <cargo build-sbf args>]`: artifacts in
  `target/solex/<channel>/`; prints the features enabled. `--all` builds
  every channel. Every command taking `--channel c` also accepts `--c`.
- `solex deploy --channel c [--cluster x] [--build] [--dry-run] [-y] [-- <args>]`:
  uses the channel's id, cluster and authority. In a terminal it shows the
  channel's clusters as a checklist to deploy to (`--cluster` limits it to
  one, `-y` skips the prompt for CI); `-- <args>` reach `solana program deploy`.
  Always show the user the dry run first. If the CLI rejects the artifact's
  `sbpf_version`, the local Solana CLI is older than the `--arch` the
  manifest builds for: upgrade it, or pass `-- --skip-feature-verify` when
  the cluster is known to run that version.
- `solex terminate --channel c [--cluster x] [--buffers]`: prints the `solana program close`
  plan; only `--yes` sends it, and mainnet also needs `--allow-mainnet`.
  Terminating a program is permanent. Never pass `--yes` unless the user asked.
- `solex env --channel c [--format dotenv|json]` and
  `solex exec --channel c -- <cmd>`: select a channel for any other build
  pipeline (sbpf-linker, solana-verify, CI). Solex owns `SOLEX_CHANNEL`,
  `SOLEX_MANIFEST`, `SOLEX_FEATURES` and, with per-cluster overrides,
  `SOLEX_CLUSTER`.
- `solex keys list | new --channel c [--cluster x] | check`: channel keypairs under `keys/`.
- `solex test --channel c -- <cargo test args>`: tests with the channel set
  and its features enabled.
- `solex setup [--no-skill] [--no-mcp]`: install this skill and register the
  MCP server with Claude Code and Codex.

Channel selection everywhere: `--channel`, else `SOLEX_CHANNEL`, else
`default_channel` in the manifest.

# MCP tools (when `solex mcp` is registered)

- `channels({manifest?})`: same data as `solex channels --json`.
- `features({manifest?, channel?})`: the matrix as JSON.
- `build({channel, manifest?, dry_run?})`: build one channel; `dry_run` returns
  the command. Builds take minutes; tell the user before running one.
- `deploy_plan({channel, manifest?})`: the exact `solana program deploy`
  command and inputs. It never deploys; the user runs it.

# References

- [Manifest schema and embedding in Anchor.toml / Quasar.toml](references/manifest.md)
- [Hand-rolled dispatch (Pinocchio) pattern](references/pinocchio.md)
- [Promotion and rollback checklist](references/promotion.md)
