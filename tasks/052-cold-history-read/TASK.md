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
- Whitespace and workspace rustfmt checks pass. Final lint/test/doc outcomes are recorded below.
- Agent reference `f36a87366d8e2fe0a1c8216463887902e471c739` was inspected at `src/store/history.rs::scan_history_page`. Its catalog/cache/scan-limit machinery is not imported: this Host already has the strict storage parser and one Session disk owner. TUI reference remains `3beeb1d8a525e58ce6352e05061f242c5580e248`, unchanged and not tested here.

## Fixed-head verification

Implementation checkpoint: `98ce59bd327b54a903e7b2e458c07c745f52b61e`.
All commands below use Rust `1.90.0`, `RUSTUP_HOME=/workspace/shared/rustup`,
`CARGO_HOME=/workspace/shared/cargo`, the Cargo bin directory first in PATH,
`TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_BUILD_JOBS=2`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`,
and dedicated `CARGO_TARGET_DIR=/workspace/shared/minicore-host-cold-history-target`.
Evidence directory: `/workspace/shared/minicore-host-cold-history-artifacts`.

- `cargo test -p minicore-host-core --lib --locked task052 -- --nocapture`: **18 passed** (`final-focused-18.log`). Both disk orderings use deterministic gates; the disk-read gate is inside the real JournalHistory mutex immediately before the real SessionStore read. The load gate is after actual exclusive journal acquisition and before owner publication. Tests also cover an actual Host and actual Runtime with a named scripted model, another Session progressing independently, late owner history, original failed saves, pending publication, timeout, disconnect after entering the publication wait, aborted disk readers, invalidated sources, cold corruption/partial tails, missing files, complete final lines without newline, read-only access, internal caller identity, unchanged queues/pause and file-free memory reads.
- `./scripts/check-whitespace.sh` and `cargo fmt --all -- --check`: passed, including a final check after code commit (`format-final.log`, status 0).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed (`clippy-workspace.log`).
- `cargo test --workspace --all-targets --locked`: **failed once**, exit 101 (`workspace-tests-once.log` and `.status`). It executed 617 unique cases: 615 passed and the two unchanged transport assertions below failed. All 248 core unit/integration cases passed, including existing pending partial, failed save and retained-history tests. Nested storage subprocess output is not counted again. No aggregate retry was made.
- `cargo test --workspace --all-targets --locked -- --list`: **742 unique cases** (`test-inventory.log`, `test-inventory-count.json`). This is an inventory, not a pass claim.
- `cargo test --workspace --doc --locked`: **4 passed** (`doctests.log`).
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`: passed (`rustdoc.log`).
- `cargo metadata --locked --offline --format-version 1`: the sole Runtime dependency remains exactly `0.10.0`, Git revision `69faf9196991524b5549499d6208a1eaf567bf56` (`metadata.json`). SPEC SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

### Preserved aggregate failure and unreached targets

The aggregate stopped at `crates/host-transport/tests/adapters.rs`:

1. `receipt_lookup_faults_are_not_method_unavailable`, line 747: actual `Null`, expected business code `1401`.
2. `rpc_protocol_errors_and_notifications_do_not_dispatch_messages`, line 418: actual `Null`, expected `HOST_METHOD_UNAVAILABLE`.

Both request `agent.start` with `{}` and still assert that the registered method is unimplemented. The fixed baseline already dispatches `agent.start` through `AgentOperation`; its missing required parameters produce the protocol invalid-params response before an owner call. Transport sources and assertions are unchanged in this topic. The coordinator confirmed the same pre-existing composition issue and owns its separate correction. This is not the known inherited-descriptor lease failure, nor evidence that the lease issue was fixed.

The failure left 125 cases unreached by that aggregate. They were then run once in separate package-selected invocations, without rerunning the failing adapters or the aggregate:

- Transport: 94 cases passed. Explicit `--test` targets were `compaction`, `compaction_wait`, `compaction_wire`, `control_observation`, `history_partials`, `interaction_answers`, `observation_order`, `observation_shutdown`, `observation_tools`, `observations`, `public_controls`, `public_controls_races`, `public_controls_storage`, `public_controls_wait`, `reload`, `reload_network`, `reload_wait`, `resource_queries`, `typed_validation_budget`, `unsubscribe_validation`, `wait_budget`, `workspace_queries`, `workspace_sources`, `workspace_tool_changes`, `workspace_wait`, `wss`, and `wss_wait` under `cargo test -p minicore-host-transport --locked`. Log: `unreached-transport.log`.
- `cargo test -p minicore-host-workspace --all-targets --locked`: 31 cases passed (`unreached-workspace.log`).

Exact argument arrays and exit statuses are in `unreached-checks.json`; per-run unique case accounting is in `executed-counts.json`. All 742 inventoried cases have now been exercised across these invocations, with 740 passing and the two preserved stale assertions failing. Package-selected runs are not a successful full-workspace aggregate.

## Remaining work and boundaries

Independent fixed-head review and coordinator composition remain pending. The coordinator maintains PROJECT.md and the unrelated transport-assertion correction. No change to storage implementation, manifests, Cargo.lock, SPEC, TUI, history-save acceptance, compaction identities or remote Git was made. No live-provider, TUI or independent effective worker-runtime validation is claimed. Original save ownership and query capability are preserved; this topic does not claim that explicit file unlocking alone resolves the reader/load race or that the separate final-owner lease defects are closed.
