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

Pending implementation and coordinator bootstrap.
