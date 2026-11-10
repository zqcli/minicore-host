# TASK079: Diagnose Host and Agent ownership boundaries

## Scope and recovery provenance

- Fixed source for this reconstruction: `8c5fe493f9b9e78e15ef831c532bc267d0c7fb53`. Branch: `task/079-host-agent-diagnostics-restored`. Worktree: `/workspace/shared/minicore-host-task079`. No push is performed by this worker.
- SPEC v4.93 SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`. Runtime remains exactly `69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0. Cargo.lock and TUI are unchanged. TUI reference remains `3beeb1d8a525e58ce6352e05061f242c5580e248`, not Runtime HEAD.
- The previous workspace disappeared before these local commits were published: first slice `b72a5f8`, span fix `71d276b4d1481ae32b9a02bf54c81d47b1379db6`, repeated-shutdown fix `e166dec8cb7d29537e045c7a121cb48aad66d01b`. Their objects and raw logs are unavailable. This is a new reconstruction from retained code-writing commands and source output, not an assertion of whole-tree byte identity or a transfer of old test passes.
- Requested worker configuration: gpt-6-astra with max reasoning. Effective settings are not independently attested.
- This worker owns Host and Agent source plus new TASK079 tests. TASK053 owns the common scalar Record helper and all Session files. The delegated-send bridge in host/collaboration.rs is also TASK053-owned. PROJECT.md belongs to the parent integrator.

## First slice

- Emit Host started only after the real memory constructor or persistent startup reaches Running.
- Agent and Session creation events are at their actual direct-owner publication boundaries. A created Agent does not imply a created default Session or delivered input.
- Preserve original operation/message/in_reply_to and Fault identity. Omit unpublished IDs. Distinguish Session metadata save from later Agent default-reference save failures.
- Record resource refusal 3403 as WARN; a Host forwarding the child's failure is not another main ERROR.
- Separate accepted stop/shutdown from completed stop/shutdown. Records contain only scalar metadata. No body, Role, config, key, script, output, path or raw exception is logged. Existing logging-health is retained without owning or closing the process writer.
- The tests use actual Host/Agent/Runtime/storage with explicitly named deterministic model and validation fixtures. Filesystem failures are real production save refusals, including a writable Session subtree with a read-only Agent parent to fail the later default-reference save. No extra recovery entry or shutdown retry is introduced.

## Known review findings at first-slice checkpoint

This initial reconstruction intentionally preserves the two independently identified defects so subsequent corrections remain small and reviewable. It is not approved for final integration on its own:

- P1: unused/by-value tracing Span destruction can invoke Subscriber::try_close while the outer admission mutex is held, on stopped-Agent creation refusal or repeated Host shutdown.
- P2: a persistent Host already Stopped can log a new Force shutdown transition as memory because its store has been retired.

TASK082 independently reconstructed its three original failing probes at `/workspace/shared/task082-review-recovery/task082_owner_review.rs`, SHA-256 `455a3a25a4b212c913cbdd05e23ddbd8a23d5a757cde245fb7f0b16f1c996b6d`. The three test bodies and subscriber checks are unchanged; its prior unrelated cases and unused imports were omitted. We will replay them on fixed new source before and after correction, not substitute old results.

## Cross-owner bridge

The four-file Host/Agent close/delete bridge was reconstructed byte-for-byte: `/workspace/shared/recovery-079-second/task079-session-record-bridge.patch`, SHA-256 `e439b94e2f45ba4cd9c8702d62cc371fc9b05fa6f50d83d5effd0f8b3b008c14`, matching the recorded original hash. `git apply --check` passed against 8c. TASK053 combines it with its Session return-record changes in one compilable independent scope. It is not mixed into this first slice.

## New environment validation

Commands use `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, `PATH=/workspace/shared/cargo/bin:$PATH`, `TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, and isolated `CARGO_TARGET_DIR=/workspace/shared/minicore-host-target079`. Dependency resolution stays `--locked --offline`.

Results below are new runs only. Full-workspace validation, independent approval and publication are not implied by focused checks.

First reconstructed slice checks:

- `cargo test -p minicore-host-core --test task079_diagnostics --test host_shutdown --test entity_retirement --locked --offline`: passed, 4 + 10 + 8 = 22 cases. Raw log: `/workspace/shared/recovery-079-second/slice1-tests.log`.
- `cargo fmt --all --check` and `git diff --check`: passed.
- The exact support fixture matches the independently restored 204-line source hash `179442c3a4e089e149355c464068a77cd5e8e4c1975ccfb01cda301c75ff8dc8`.
- Known P1/P2 remain at this checkpoint. Clippy and broader regressions are still due on the corrected source.

## P1 correction after new red reproduction

On fixed reconstructed `69826db8cb3efb89648d1f38f5dcef2503c69d05`, the unchanged recovered TASK082 three-probe file (SHA-256 above) was copied to a separate worktree and executed with `cargo test -p minicore-host-core --test task082_owner_review --locked --offline -- --nocapture --test-threads=1`. All three failed as expected: both reentry cases observed try_close under admission and the repeated-force case observed false memory mode. Exit 101 and raw output are retained in `/workspace/shared/recovery-079-second/probes-red-69826db.log`. The test source hash remained unchanged.

P1 now keeps `Option<Span>` in the outer Host/Agent caller. Acceptance only borrows it and takes it on successful transfer to an owned task. Rejected admission and repeated shutdown destroy the unused span after the outer guard returns. No Drop emitter, owner state, new acceptance, retry or business mutation was added.

- `cargo test -p minicore-host-core --test task079_lock_boundary --test task079_diagnostics --locked --offline`: passed, 2 unchanged reentry probe bodies plus 4 diagnostics cases.
- `cargo clippy -p minicore-host-core --lib --test task079_lock_boundary --test task079_diagnostics --locked --offline -- -D warnings`: passed.
- `cargo fmt --all --check` and `git diff --check`: passed.
- Raw logs: `/workspace/shared/recovery-079-second/p1-tests.log`, `p1-clippy.log`.
- P2 remains at this commit; independent post-fix review is pending.

## P2 correction

A repeated request after Stopped still returns and retains its real receipt, including a requested Force strategy. It does not start another shutdown task or transition out of Stopped. Therefore it no longer creates a second `host.shutdown_accepted` lifecycle transition. This preserves the existing “repeated request returns current facts” behavior rather than hiding a real new acceptance. No persistent mode is guessed from an already released store, no immutable mode field is added for logging, and original business strategy/receipts are unchanged. The added regression asserts both receipt retention and exactly one original persistent shutdown transition.

New corrected-source checks:

- `cargo test -p minicore-host-core --test task079_shutdown_repetition --test task079_lock_boundary --test task079_diagnostics --test host_shutdown --test persistent_owners --test entity_retirement --locked --offline`: passed, 1 + 2 + 4 + 10 + 46 + 8 = 71 cases.
- `cargo clippy -p minicore-host-core --lib --test task079_shutdown_repetition --test task079_lock_boundary --test task079_diagnostics --locked --offline -- -D warnings`: passed.
- `cargo fmt --all --check` and `git diff --check`: passed.
- Raw logs: `/workspace/shared/recovery-079-second/p2-regression.log`, `p2-clippy.log`.
- Unchanged three-probe replay will also run on the resulting fixed commit in a separate checkout. The independent TASK082 decision is still pending; this is not a full-workspace pass.

Unchanged external probe replay completed on fixed new commit `3bd6f054bc47076c6e9a2590df8a8edffc0515f7` in a separate checkout: `cargo test -p minicore-host-core --test task082_owner_review --locked --offline -- --nocapture --test-threads=1` passed all 3 cases, exit 0. The source hash before and after is `455a3a25a4b212c913cbdd05e23ddbd8a23d5a757cde245fb7f0b16f1c996b6d`, identical to the reconstructed reviewer artifact used in the new red run. Raw output: `/workspace/shared/recovery-079-second/probes-green-3bd6f05.log`. Independent review remains separate from this producer replay.

## Remaining scope

The previous environment's uncommitted metadata save/reconcile and additional reclamation instrumentation was lost. It is not part of these restored commits and its old focused results are not current evidence. That work must be reconstructed and tested as a later independent slice after synchronizing with the shared close/delete bridge. No new recovery entry, automatic shutdown retry or lifecycle framework is planned.
