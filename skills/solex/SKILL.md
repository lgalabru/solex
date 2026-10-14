---
name: solex
description: |
  Release channels for Solana programs with solex: ship one codebase as stable / nightly / canary deployments, declare Cargo features per channel in Solex.toml and gate code with #[cfg(feature = ...)], epoch-gate activation, keep per-deployment constants in the manifest, and build/deploy per channel.
  TRIGGERS: "feature gate this instruction", "ship to nightly first", "release channel", "what ships on mainnet", "solex", "Solex.toml", "Solex.yaml", "#[channel]", "[features.", "promote to stable", "devnet-only instruction", "epoch activation", "deploy per cluster", "channel-gated build"
  Use when adding, gating, promoting or auditing program features across deployments, or when a program has cfg/feature flags that select clusters or program ids.
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
increments that reach nightly first, then stable, are testable and auditable
as one diff each. The stable artifact stays small.

# Core workflow

1. `solex channels` to see the deployments and the features each enables.
   If there is no manifest, `solex init <name>` or add a `[solex]` section.
2. Declare the feature once: `hooks = []` in `Cargo.toml` `[features]`, and in
   the manifest:
   ```toml
   [features.hooks]
   description = "V2 lifecycle hooks"
   channels = ["devnet"]
   ```
3. Gate every piece of the feature with `#[cfg(feature = "hooks")]`: enum
   variants, `match` arms, handlers, discriminators, accounts, state. A
   missing marker is a normal compile error.
4. `solex features --check`: one row per feature with a tick per channel, the
   names behind it, and errors for a feature missing from `Cargo.toml`,
   declared but unused, or a `#[channel]` tag that is not a channel. Paste
   the table into the PR.
5. `solex build --channel devnet`, `solex test --channel devnet`,
   `solex deploy --channel devnet`.
6. Promote in the manifest: add `mainnet` to `channels = [...]` (or move the
   feature to a channel that mainnet includes). The source does not change.

# Rules

- One feature, one declaration. Never repeat "where" in the source; the
  manifest owns it. Prefer a feature over `#[channel(nightly)]` tags unless
  code is genuinely tied to a release train.
- Gate the public surface, not shared checks. Validation and version checks
  used by stable paths stay unconditional.
- Activation epochs live in the manifest: `epoch = { mainnet = 850 }` on the
  feature, plus `#[solex::activation("hooks")]` on the handler. It is a no-op
  on channels without an epoch and returns `FEATURE_NOT_ACTIVE`
  (`ProgramError::Custom(0x53580000)`) before the epoch elsewhere.
- Per-deployment constants (treasury, admin, fee accounts) go under `vars` on
  each channel and are read with `solex::var!("name")`. A channel missing the
  var fails to compile; never keep a placeholder constant in code.
- `cluster = ["mainnet-beta", "devnet"]` deploys one channel to several
  clusters; `[channels.<channel>.<cluster>]` overrides `program_id` or `vars`
  for one of them, and the channel then builds once per cluster.
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
  row per feature; `solex features hooks` lists that feature's markers with
  file:line; `--items` lists all markers.
- `solex build --channel c [-- <cargo build-sbf args>]`: artifacts in
  `target/solex/<channel>/`. `--all` builds every channel.
- `solex deploy --channel c [--build] [--dry-run]`: uses the channel's id,
  cluster and authority. Always show the user the dry run first.
- `solex terminate --channel c [--buffers]`: prints the `solana program close`
  plan; only `--yes` sends it, and mainnet also needs `--allow-mainnet`.
  Terminating a program is permanent. Never pass `--yes` unless the user asked.
- `solex env --channel c [--format dotenv|json]` and
  `solex exec --channel c -- <cmd>`: select a channel for any other build
  pipeline (sbpf-linker, solana-verify, CI). Solex only owns the two
  environment variables `SOLEX_CHANNEL` and `SOLEX_MANIFEST`.
- `solex keys list | new --channel c | check`: channel keypairs under `keys/`.
- `solex test --channel c -- <cargo test args>`: tests with the channel set
  and its features enabled.
- `solex setup [--mcp]`: install this skill; register the MCP server.

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
