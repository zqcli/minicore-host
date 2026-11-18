# Task 120: Align Host and TUI versions to 0.12.8

## Scope and baselines

The owner requested one shared version, 0.12.8, with no functional changes.
This task prepares local commits only. Do not push, create tags, merge to main
or publish a release as part of this task.

- Host `dev`: `02c7558b7eff324f2f83557305d921912a4dce74`, version 0.12.6.
- TUI `dev`: `783ad1d954e74ff77afbabb0df6ac8f339a44829`, version 0.12.7.
- SPEC 4.11.5 SHA-256: `4459dcbf395552e7e6bdab94ec322a93391d8a5a7d0721f1947afbfa7b929e2c`.
- Requested engineering/review configuration: `gpt-6-astra` with `max` reasoning;
  this is not independent runtime attestation. No Codex task or CLI is used.

## Changes

All fourteen Host workspace packages, thirteen shared internal path requirements
and the storage crate's exact protocol requirement become 0.12.8. The lockfile
changes only the fourteen internal package versions. Current README, CI and
configuration references reflect 0.12.8 and the existing SPEC 4.11.5; historical
reports and the specification itself remain unchanged.

TUI uses 0.12.8 for its own package and every Host requirement/lock record. Its
immutable Git source must point to the final Host commit for this task. Current
docs and four version-bearing snapshots change their version labels only; a
new changelog entry preserves all previous release entries.

All Rust source files, external dependency records and native storage formats
remain unchanged. The independent Web client is outside the task.

## Validation

On Linux with the repository-pinned Rust 1.90.0 and an isolated writable HOME:

- `./scripts/check.sh`: passed. This includes whitespace, all 28 Python checks,
  formatting, strict workspace/all-target Clippy, every Rust test, doctests and
  strict workspace Rustdoc, using the committed lockfile.
- `cargo test --workspace --all-targets --locked --offline -- --list`: passed;
  1,263 actual cases in 148 test targets, with no ignored cases. Nested storage
  subprocess output is not counted again.
- `cargo test --workspace --doc --locked`: all seven doctests passed as part
  of the complete gate above.
- `cargo metadata --no-deps --locked --format-version 1`: passed; all fourteen
  workspace packages report 0.12.8.
- Structural lock comparison: all 225 complete external package records are
  identical; internal records differ only in their version.
- Independent read-only review: all 470 Rust files, the specification and all
  223 existing historical task files are unchanged; no blocking finding.
- `git diff --check`: passed after the final documentation update.

The TUI quality, snapshot and release-version checks run against the fixed Host
commit produced here and are recorded by the coordinator with the frontend
commit. No new remote CI, macOS/Windows execution, platform packaging, live model
or physical terminal acceptance is claimed by this local Host gate.

## Publication order

After separate publication authorization, push the exact Host commit first,
verify its remote SHA, then push TUI with that same immutable Host pin. If the
publication mechanism creates a different Host commit, update every TUI Host
revision in the manifest, lockfile and new changelog entry and repeat the exact-pin
checks before publishing TUI. Do not force-push or rewrite existing remote history.
