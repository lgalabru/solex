---
name: solex
description: |
  Release channels for Solana programs with solex: ship one codebase as stable / nightly / canary deployments, gate instructions with #[channel(...)], epoch-gate activation, keep per-deployment constants in the manifest, and build/deploy per channel.
  TRIGGERS: "feature gate this instruction", "ship to nightly first", "release channel", "what ships on mainnet", "solex", "Solex.toml", "Solex.yaml", "#[channel]", "promote to stable", "devnet-only instruction", "epoch activation", "deploy per cluster", "channel-gated build"
  Use when adding, gating, promoting or auditing program features across deployments, or when a program has cfg/feature flags that select clusters or program ids.
---

# solex: release channels for Solana programs

A channel is a deployment: its own program id, cluster and upgrade authority,
declared once in `Solex.toml` / `Solex.yaml` or under `[solex]` in `Anchor.toml`.
`#[channel(tag)]` compiles an item only into channels that ship that tag.
Everything untagged ships everywhere. A build contains exactly one channel's
features; nothing dormant is left in the binary.

Why it matters: agents produce more code than a big release can absorb. Small
increments that reach nightly first, then stable, are testable and auditable
as one diff each. The stable artifact stays small.

# Core workflow

1. `solex channels` to see the deployments and what each ships (`includes`).
   If there is no manifest, `solex init <name>` or add a `[solex]` section.
2. Write the change. Tag anything not ready for every channel with the channel
   it belongs to: `#[channel(nightly)]` on the instruction handler, its accounts
   struct, and any new state, each with the same quoted feature name. Leave
  stable code untagged.
3. `solex features --check` to see the item × channel matrix and catch tags no
   channel defines. Paste the table into the PR description.
4. `solex build --channel nightly` then `solex deploy --channel nightly`.
5. Promote by changing the channel tag on every marker of the feature
   (`solex features` shows the count and flags a marker left behind), or
   remove the tag so the feature ships everywhere. Never copy code paths.

# Tagging rules

- Name the feature in the attribute: `#[channel(devnet, "hooks")]`. The
  channel says where it ships, the quoted name groups every marker of that
  feature (variants, arms, handlers, consts) into one row of `solex features`
  and one line of `solex channels`. Use the same name on all of them.
- A channel `includes` the channels whose tags it also ships, so `nightly`
  typically `includes = ["stable"]`. Clusters are channel properties, never tags.
- `cluster = ["mainnet-beta", "devnet"]` deploys one channel to several
  clusters. When a cluster needs its own `program_id` or `vars`, add
  `[channels.<channel>.<cluster>]` with just the fields that differ; the
  channel then builds once per cluster (`target/solex/<channel>/<cluster>/`)
  and every command accepts `--cluster` to pick one. Tag-only channels (no
  `program_id`) exist to be `include`d and are never built.
- Untagged code is the stable surface. Tag only what is new or experimental.
- Gate the public surface: instruction variants, dispatch arms, handlers,
  accounts structs, new account types, new error variants. Do not gate
  validation or security checks shared with stable paths.
- Attribute macros cannot sit on enum variants or `match` arms. Put
  `#[solex::gated]` on the enum, fn or impl and use `#[channel(...)]` on the
  variants/arms inside. `#[solex::program]` does this for Anchor modules.
- `#[channel([nightly, 850])]` compiles into nightly and returns
  `FEATURE_NOT_ACTIVE` (`ProgramError::Custom(0x53580000)`) until epoch 850.
  Use it to ship code ahead of a coordinated activation, like Solana feature
  gates. Epoch bounds are valid on functions only.
- Per-deployment constants (treasury, admin, fee accounts) go under `vars` on
  each channel and are read with `solex::var!("name")`. A channel missing the
  var fails to compile; never keep a placeholder constant in code.
- Outside a `#[solex::program]` or `#[solex::gated]` item, spell the attribute
  `#[solex::channel(...)]` or `use solex::channel;` first.

# Before promoting to stable

- Did nightly run the instruction on devnet with real clients? Link the
  transactions in the PR.
- Does promotion create a new account layout? Once stable accounts exist, a
  rollback to a binary without that layout strands them. Promote state
  formats deliberately and document the rollback boundary.
- Does the IDL for stable change? Gating removes instructions from the binary;
  regenerate and diff the IDL per channel.
- Run `solex features --json --channel stable` and confirm only intended rows
  show `ships: true`.

# CLI

- `solex channels [--json]`: deployments, ids, clusters, vars, resolved tags.
- `solex features [--channel c] [--items] [--json] [--check]`: what ships
  where, one row per gate; `--items` shows every marker with file:line.
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
- `solex test --channel c -- <cargo test args>`: tests with the channel set.
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
