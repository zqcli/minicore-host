# Coordinate cold history reads with the Session owner

## Fixed baseline and design freeze

- Base: `accc5ddd120444cabe90c4756f5efa2bb047392c`.
- Topic: `fix/cold-history-read-load`; worktree: `/workspace/shared/minicore-host-cold-history-read`.
- Own `crates/host-core/src/backend.rs`, `session/history_read.rs`, new task052 tests and this task evidence. Common trait/export/Session/control declarations are supplied by the coordinator. No storage, manifest, SPEC, TUI or publication changes.
- Runtime stays exactly `0.10.0` at `69faf9196991524b5549499d6208a1eaf567bf56`; retain Cargo.lock. Requested worker configuration is Astra/max; no independently verified effective-runtime claim.
- Read AGENTS.md and SPEC sections 4, 5, 8 and 9. The independent normal Host probe already records `history.read` failing with “history already has an active writer” while explicit load succeeds. This is separate from inherited descriptor/final-owner unlocking.

Minimal design agreed before implementation: add a default, non-mutating `HistoryBackend::read_cold` hook returning `Result<Option<Vec<HistoryEntry>>, Fault>` in `HistoryReadFuture`. `Some` means a strictly validated cold disk snapshot; `None` delegates to the existing Session load/save owner. Unsupported custom backends return an explicit read failure, never invoke load. JournalHistory uses its existing disk-only mutex to serialize the cold read with load/append/release. An existing journal or pending original save returns `None`, never `loaded_records` as a current cache.

Session history.read checks actual retained history first and again after I/O. If load won and publication is pending, it waits on existing Session state notifications, registered before checking state. It rechecks access and source lifetime before success or failure; external revocation ends only that caller's wait. No owner/access lock crosses I/O, no new Session coordination state, retries, cache, service, Runner, input consumption, pause changes, writes or save reconciliation. Driver-side successful load must publish before potentially slow resource preparation so waiting readers can see actual owner-retained content.

## Verification plan

Deterministic gates cover load winning before publication, read winning under the disk mutex, newer retained data, unrelated Session progress, cancelled/revoked waits and source reclamation. Real temporary journals and production Host/Runtime fixtures cover cold persisted reads, corruption positions, partial tails, unchanged bytes, read-only callers, retained/unconfirmed content and memory remaining file-free. Only test-owned temporary data may be modified. Record exact commands/results below; do not rerun aggregate lease failures until green.

## Results

- Coordinator bootstrap: `f32013ea2c935beeb12a3b7d8d16267b92e6905b` (trait/export and successful driver load publication).
- Deterministic red regression: `c4c98a2471354089076a7051518195c9fd8272e6`. With the original content-read implementation, the test fails with `SESSION_HISTORY_LOAD_FAILED: history already has an active writer`. Evidence: `/workspace/shared/minicore-host-cold-history-artifacts/baseline-red.log` and `baseline-red-test.rs`.
- Implementation now coordinates cold reads with the existing disk mutex and waits for actual Session publication. The optional `CallerContext::read_revocation` accessor was separately approved by the coordinator; it creates no new channel/state and does not change access decisions.
- Eighteen focused task052 cases pass, including production Host and Runtime with a named scripted model. See `final-focused-18.log` in the evidence directory. The expanded test fixture initially omitted forwarding `reconcile`; that fixture-only failure is preserved in `fixture-reconcile-forwarding-failure.log` and was corrected by forwarding the existing original-save method. No production retry behavior changed.
- Whitespace and workspace rustfmt checks pass. Full lint/test/doc gates are pending below.
- Agent reference `f36a87366d8e2fe0a1c8216463887902e471c739` was inspected at `src/store/history.rs::scan_history_page`. Its catalog/cache/scan-limit machinery is not imported: this Host already has the strict storage parser and one Session disk owner. TUI reference remains `3beeb1d8a525e58ce6352e05061f242c5580e248`, unchanged and not tested here.
