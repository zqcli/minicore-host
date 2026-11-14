# Task 114: merge upstream and integrate Host 0.12.1

## Scope and checkpoints

- Exclusive worktree: `/Users/zzq/Develops/minicore-host-wt/sync-0121`.
- Branch: `sync/upstream-0121`; local parent `b340c0bf8e624fc66b5123f9f999d58206bcd60c`.
- Fetched upstream parent: `21c7ff1610c8c7000694df2d141c62a74790e4be` (0.12.0).
- Integration uses a two-parent merge, without rebase/reset, push or tag.
- Baseline: SPEC 4.11.0, SHA-256 `0fe877e0d2809fd6a11b44ff8f90c6aa9029446f3757ea26d393a2ebed9ef016`.
- Worker configuration requested by project: `gpt-6-astra` / `max`; effective runtime settings are not independently verified. No subagents were used.

## Merge decisions

Only `PROJECT.md` and `crates/host-resources/src/resolver.rs` conflicted.
The resolver retains the explanatory local comment and exactly one shared
name sort; both local ordering regressions remain. The Responses resource
integration test auto-merges upstream configuration changes with the local
manual-compaction regression. Provider implementation and regression files
retain the local done-only, R3/R4/R5 and cross-model repairs unchanged.
Upstream JSON configuration directories and Host directory selection remain.
PROJECT retains both local and upstream evidence with a separate current status.
The distinct old Task 110/111 directories are preserved.

Version scope: 14 inherited workspace package versions, 13 root internal
dependency declarations plus storage's exact protocol dependency, and all 14
internal Cargo.lock packages become 0.12.1. Only current-version README and
configuration/CI guide references change; historical version records do not.
No third-party upgrade, compatibility framework or deferred R1/R2 repair.

## Verification

All Rust/Cargo/rustfmt commands run through the authorized runner, never on macOS:
`python3 /Users/zzq/Develops/minicore-remote-build/sync-0121/remote-check.py /Users/zzq/Develops/minicore-host-wt/sync-0121 LABEL 'COMMAND'`.

Logs live in `/Users/zzq/Develops/minicore-remote-build/sync-0121/evidence/`.

- `host-host114-lock.log`: `cargo metadata --format-version 1 --no-deps`
  confirms 14 members at 0.12.1; subsequent `cargo check --workspace --locked`
  exits 101 because the version change requires a lock update. This is retained
  as an unsuccessful preliminary check, not reported as a passed gate.
- `host-host114-resolve.log`: `cargo metadata --offline --format-version 1`
  updates only 14 internal package versions. Exit 0. Complete external lock
  entries (225, including versions, sources, checksums and dependency lists)
  are identical before/after and independently compared against both parents.
- `host-host114-gate.log`: full gate exits 0:
  `setpriv --bounding-set=-dac_override,-dac_read_search ./scripts/check.sh`.
  Whitespace checks, 28 Python checks (15 + 5 + 8), formatting, strict Clippy,
  all workspace/all-target tests, 7 doctests and strict rustdoc pass. No failed
  or ignored cases are reported. Root DAC bypass capabilities are removed for
  the actual permission-sensitive tests; no assertions are changed.
- `host-host114-list.log`: independent enumeration exits 0:
  `cargo test --workspace --all-targets --locked -- --list`.
  Exactly 1,234 unique `(test executable, test name)` cases across 148 target
  listings, matching the sum of listing summaries. Nested storage subprocess
  output is not counted. This is 18 cases beyond upstream's recorded 1,216.

[validation.json](validation.json) records log SHA-256 values and exact scope.
No source/manifest/lock changes follow these passing checks; only completion
records are finalized before the single integration commit.

Local source comparisons verify provider source/tests equal the local parent;
configuration, core, workspace, transport, app and protocol code equal upstream.
Resolver changes against the local parent are only the two upstream Agent
instruction arguments. The resource Responses test changes against that parent
only adopt the upstream reload result's Agent outcomes field. The lock diff
against upstream is exactly 14 version-line substitutions.

Local staged/working-tree whitespace is checked before commit. The runner's
scratch Git HEAD initially predates this integration: the full gate checks that
history plus the synchronized working tree, not the final integration commit.
The coordinator will synchronize final Git objects and verify final committed
whitespace; no global safe.directory, identity or hooks are changed.

## Commit identity

The final integration is the single merge commit with subject
`chore(integration): merge upstream and release Host 0.12.1`, whose ordered
parents are the full local and upstream SHAs above. Its exact SHA is returned
in the final handoff rather than creating a self-referential documentation
commit. All retained historical Task 110/111 records keep their original names.

## Limits and handoff

No main worktree/dev operations, TUI edits/startup, live model calls, real user
configuration/session access, credentials, publication or new native-platform
validation. Final Host SHA is returned to the coordinator for an immutable TUI
pin; no commits will be appended after that handoff.
