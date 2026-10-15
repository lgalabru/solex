# Promotion and rollback

## Promote canary → stable

1. Evidence: devnet transactions exercising the feature, client parity tests,
   `solex features --json --channel canary` showing it enabled.
2. Edit the manifest: add the target channel to `[features.<name>] channels`
   (or move the feature to a channel the target includes). No source change.
3. `solex features --check`, then build every channel: `solex build --all`.
4. Regenerate the stable IDL and clients; the diff should be exactly the
   promoted instructions.
5. `solex deploy --channel stable --dry-run`, review, then deploy.

## Epoch-gated activation

Ship early: list the channel under the feature and give it an epoch,
`epoch = { stable = N }`, with `#[solex::activation("<name>")]` on the
handlers. The binary is deployed and auditable before activation; calls fail
with `FEATURE_NOT_ACTIVE` until epoch N. Remove the epoch once active.

## Rollback boundaries

- Instructions: removing a gated instruction from a channel is safe as long as
  no client depends on it there.
- Accounts: once a channel has created accounts in a new layout, a binary that
  does not understand that layout strands them. Keep version-aware loading
  untagged and treat state-format promotion as one-way.
- Program ids: channels are separate deployments. Accounts at one channel's
  address are never valid at another's; promotion moves code, not state.

## Other build pipelines

`solex build` is `cargo build-sbf` with `SOLEX_CHANNEL` and `SOLEX_MANIFEST`
set. Configure `[build] command` / `artifact` in the manifest to let it drive
sbpf-linker or a custom script, or export the variables yourself with
`solex env` / `solex exec`. Verified builds (`solana-verify`) must receive the
same two variables inside the container.

## CI

- `solex features --check` fails the build on unknown tags.
- Build the matrix: `solex build --all`, upload each `target/solex/<channel>/*.so`.
- Compare the stable artifact against the previous release; the size delta
  should match the promoted code only.
