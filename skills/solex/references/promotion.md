# Promotion and rollback

## Promote nightly → stable

1. Evidence: devnet transactions exercising the instruction, client parity
   tests, `solex features --json --channel nightly` showing it shipped.
2. Change the tag. Remove `#[channel(nightly)]` so the item ships everywhere,
   or retag to `#[channel(stable)]`. Do not duplicate code paths.
3. `solex features --check`, then build every channel: `solex build --all`.
4. Regenerate the stable IDL and clients; the diff should be exactly the
   promoted instructions.
5. `solex deploy --channel stable --dry-run`, review, then deploy.

## Epoch-gated activation

Ship early with `#[channel([stable, N])]`, where N is the activation epoch.
The binary is deployed and auditable before activation; calls fail with
`FEATURE_NOT_ACTIVE` until epoch N. Remove the bound in a later release once
active everywhere.

## Rollback boundaries

- Instructions: removing a gated instruction from a channel is safe as long as
  no client depends on it there.
- Accounts: once a channel has created accounts in a new layout, a binary that
  does not understand that layout strands them. Keep version-aware loading
  untagged and treat state-format promotion as one-way.
- Program ids: channels are separate deployments. Accounts at one channel's
  address are never valid at another's; promotion moves code, not state.

## CI

- `solex features --check` fails the build on unknown tags.
- Build the matrix: `solex build --all`, upload each `target/solex/<channel>/*.so`.
- Compare the stable artifact against the previous release; the size delta
  should match the promoted code only.
