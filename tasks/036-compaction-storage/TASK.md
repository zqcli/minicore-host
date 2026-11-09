# Persist cumulative compaction coverage

Status: storage implementation, focused checks and the full workspace gate pass
at the fixed checkpoint recorded below. Independent review and Session manual/
automatic ownership, cancellation and resource/routing integration remain separate.
No push was made.

## Fixed inputs and ownership

- Base: `cdb8ee144ef92ff29c1025148e05b5a9bf50f2e3`.
- Topic: `feat/compaction-history-record`, independent worktree and build target.
- Runtime: `69faf9196991524b5549499d6208a1eaf567bf56`, exact `0.10.0`, unchanged.
- SPEC v4.93: original transcript, strict supported formats, no automatic repair,
  incremental history acceptance and exact original-save reconciliation.
- Scope: `crates/host-storage/**` and this task only. Core's exhaustive readable
  history match is coordinator-owned. No manifests, Runtime, TUI or SPEC edits.
- The parent owns PROJECT.md integration and independent review.

## Reference assessment

Read the fixed Agent reference `f36a87366d8e2fe0a1c8216463887902e471c739`,
particularly `src/compaction/{projection,snapshot}.rs`, `src/store.rs` and
`src/store/history.rs`. It is a code reference, not an implementation baseline.
Reuse its separation of raw history from a derived summary and cumulative native
item coverage. Its separate `summary.json`, byte/hash/whole-Loop anchors and
snapshot fallback do not match Host's existing single JSONL save/reconcile owner.
Its append rollback and tail truncation conflict with Host's no-repair contract.
No Agent-format compatibility, caps or recovery mechanisms were copied.

## Stored expression and format policy

`HistoryEntry::Compaction { source_position, covered_items, summary }` records
actual Runtime `SummaryHistory` with cumulative coverage `[0, covered_items)` of
the untouched raw native transcript. Source is the existing Session-local next
JSONL position and must equal this marker's outer position. It is not a new ID.
Partial, TurnFinished and Compaction records do not count as native items.
Existing literal Summary keeps its exact literal-item meaning and does count.
No coverage is guessed from its text.

Object files remain schema v1. `HISTORY_SCHEMA_VERSION = 2` is used for all new
history writes. Readers explicitly accept the original v1 branches in their exact
shapes and v2 branches; Compaction requires v2. Mixed supported rows are allowed
without rewriting any old bytes. Unknown versions and malformed records fail
unchanged. This is explicit support for this repository's existing format, not a
claim that v1 compaction was deployed and not a migration/compatibility platform.

Journal admission and loading validate source, positive advancing coverage,
actual prior native length, nonblank summary and whole tool-exchange boundaries.
The existing call index gains raw call/result positions; no second full native
transcript is retained by the validator. A cut cannot leave an actual result
without its call or cover an unresolved tool-call tail. A later native non-result
group establishes the end of a historical missing-result exchange. Its missing
outcomes remain unknown. A later result cannot cross an already committed cut.
Matching uses actual native association, not globally unique call strings.

The existing append, file/directory sync, pending range and reconciliation engine
is unchanged. Validation failure drops the batch overlay before acceptance.
Storage does not install Session context, arbitrate cancellation or rerun models.

## Frozen owner helper

- `CompactionProjection { source_position: u64, covered_items: u64, summary:
  SummaryHistory }` is pure derived state with public fields.
- `new(source_position, covered_items, summary)` and `into_entry()` construct the
  marker; neither is a save confirmation.
- `validate(raw_history: &[HistoryItem], expected_source_position: u64,
  previous_coverage: u64) -> Result<(), HistoryProblem>` checks a candidate at the
  owner's current history boundary. The owner separately checks operation,
  configuration, cancellation and actual save confirmation.
- `project(raw_history) -> Result<Vec<HistoryItem>, HistoryProblem>` returns only
  the latest summary and untouched raw suffix.
- `LoadedHistory::runtime_history()` remains raw. `compaction_projection()` returns
  the latest validated marker; `model_history()` returns the derived projection.
- The owner must keep the old projection through pending/failed save and install
  the candidate only after the exact original range is confirmed.

Core's one exhaustive readable-history arm must accept
`HistoryEntry::Summary { summary } | HistoryEntry::Compaction { summary, .. }`.
The marker must never become another raw native item through `runtime_item()`.

## Verification

Environment: Rust 1.90.0, fixed Cargo.lock, two build jobs, separate target,
no incremental or debug information. Focused commands already passed:

```sh
cargo check -p minicore-host-storage --locked --offline
cargo test -p minicore-host-storage --all-targets --locked --offline
cargo clippy -p minicore-host-storage --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Current storage total: 76 unique tests (25 unit and 51 integration), including
19 new compaction tests. Nested subprocess probes are not additional cases.
The new cases use real temporary files and actual Runtime value types. Injected
write/sync failures are explicitly test hooks, not live failure claims.

Coverage includes repeated compaction/reload, unchanged original byte prefixes,
strict v1/v2 and missing/invalid fields, nonadvancing/out-of-range/source mismatch,
multi-tool and nonadjacent call/result cuts, historical missing/mixed results,
partials/finish/markers excluded from native counts, rejected-batch rollback of
validation only, memory no-files, zero/full-before-sync/partial writes, original
prefix changes and confirmation without duplicate marker append.

## Fixed checkpoint and full gate

- Storage implementation: `6d1d41cd64555d94e9a3ea8b1bfab5daf380a93b`
  (`feat(storage): persist cumulative compaction coverage`).
- Coordinator's sole core exhaustive arm:
  `2847547aabe000c3f58351d1f9fa8a7ce00c9fe2`, cherry-picked locally as
  `32623acce16d949eb001823c0ceab1bea01ed7ce`. No other core source was changed.
- The complete gate ran at `32623acce16d949eb001823c0ceab1bea01ed7ce`:

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
cargo metadata --locked --offline --format-version 1
sha256sum SPEC.md
```

All checks passed: whitespace, formatting, strict all-target workspace Clippy,
**584 unique target/test cases**, **3 workspace doctests**, and strict workspace
rustdoc. Storage contributes 76 cases, including the 19 new compaction tests.
The known task024 subprocess-lock flake did not occur; the separate test-helper
fix was not included in this checkpoint. No production lock was weakened.

Metadata confirms exactly one Runtime `0.10.0` at
`69faf9196991524b5549499d6208a1eaf567bf56`, without a patch. SPEC SHA-256 remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
No manifests or lockfile changed. The final target measured about 1.5 GiB, and
8.5 GiB free was reported at gate completion. This task deleted no other target.

Reproducibility logs are in the sibling
`minicore-host-compaction-storage-validation` directory: `focused-tests.log`,
`focused-clippy.log`, `storage-test-list.txt`, `storage-doc-format.log`,
`full-gate.log`, `workspace-test-list.txt` and `metadata.json`. The enumeration
counts test cases once by target and name, never nested subprocess output.

This gate does not establish manual/automatic Session compaction, public routing,
utility generation through Session, Runtime cancellation races or compaction
lifecycle acceptance. Those remain the Session owner's integration work. Live
providers, credentials and TUI were not exercised or claimed by this task.
