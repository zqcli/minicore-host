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

## Shutdown mode race follow-up

On the fixed integrated `985ff19457e682f4d30f6c9bb309a3cbc2d118bf` base, TASK082 found a concrete remaining interleaving: the shutdown worker releases `store` before publishing Stopped, so a concurrent Force request can observe no store while its accepted lifecycle is still Stopping. Reconstructing mode from a live resource handle could therefore emit false `memory` again. The reviewer ran 300 public-API race iterations without hitting that interval; this is static interleaving evidence, not a reproduced red test or an asserted observed failure.

The follow-up preserves the immutable construction mode in Host and reads it for shutdown diagnostics. It changes no lifecycle, strategy, receipt, worker or resource-release ordering. The earlier rule that a request against Stopped creates no new shutdown transition remains intact. This supersedes only the earlier choice to avoid retaining construction mode, now needed to report a fact independently of resource retirement.

Fresh checks in `fix/079-host-mode-diagnostics` / `/workspace/shared/minicore-host-task079-mode`, same Rust environment and isolated target as above:

- `cargo test -p minicore-host-core --test task079_shutdown_repetition --test task079_lock_boundary --test task079_diagnostics --test host_shutdown --test persistent_owners --test entity_retirement --locked --offline`: 71 passed.
- `cargo clippy -p minicore-host-core --lib --test task079_shutdown_repetition --test task079_lock_boundary --test task079_diagnostics --locked --offline -- -D warnings`: passed.
- `cargo fmt --all --check` and `git diff --check`: passed.
- Logs: `/workspace/shared/task079-mode-evidence/regression.log`, `clippy.log`. Independent TASK082 follow-up and a complete workspace gate remain separate.

## Agent metadata save and reconciliation slice

- Branch `task/079-agent-metadata-diagnostics`, worktree `/workspace/shared/minicore-host-task079-metadata`. Began from `985ff19457e682f4d30f6c9bb309a3cbc2d118bf` and fast-forwarded to independently reviewed mode fix `7781a1d8f58967dfb76d52fbef6a224f813911d6` before final checks.
- This is newly written/tested work, not byte-for-byte recovery of the lost metadata edits. The existing `AgentRecordWork` retains only an additional scalar Context, so explicit reconcile continues the original operation without inventing another identity or retry owner.
- Record actual save acceptance, confirmed commit, and failed save separately. Saving retains the old committed revision and does not claim persistence. Memory commits use `not_applicable`; only a confirmed persistent save uses `persisted`. The immutable Saving receipt remains unchanged after commit or failure.
- The direct Agent reports one main ERROR for its actual metadata/default-reference save failure. Close, Session deletion and Agent cascade retain the original operation and target Session through that same owner; forwarding does not duplicate the ERROR. Resource refusal 3403 remains WARN and a rejected update does not create a save or receipt.
- Synchronous records leave the outer admission before emit. Owned save workers create/end their short span after releasing owner guards and explicitly inherit the subscriber. No Span enters pending owner state. No body/config/Role/name/path/raw error is logged; optional IDs absent from the caller are not fabricated.
- No Session source/signature, shared helper, configuration/resource/schema, Runtime, TUI or lockfile change. No new recovery API, implicit close/shutdown retry or lifecycle state. The Host deletion change only carries its already-known operation into the existing Agent worker and propagates the subscriber.

Fresh validation with the Rust environment/isolated target above:

- `cargo test -p minicore-host-core --locked --offline`: all **317** core cases passed, zero ignored; doc-tests contain zero cases. `cargo test -p minicore-host-core --all-targets --locked --offline -- --list` independently lists 317 distinct test names. This is not a workspace-wide gate.
- Six new metadata integration cases cover real filesystem save refusal/reconcile, unchanged original receipt/revision, default-close forwarding, Session/Agent deletion forwarding, an accepted gated save after waiter cancellation and connection revocation, memory commit vs persistent commit, real resource refusal, and uninstalled/closed logging. Two new callback-reentry cases cover memory metadata commits and persistent failure/reconcile, alongside the unchanged two P1 probes.
- `cargo clippy -p minicore-host-core --lib --tests --locked --offline -- -D warnings`: passed.
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core --no-deps --locked --offline`: passed. `cargo fmt --all --check` and `git diff --check`: passed.
- The first all-core run exposed a fixture race: it closed the writer immediately after observing business publication while another Runtime poll could still emit its diagnostic tail. The fixture now shuts down its test-owned Runtime before flushing the independent writer. It does not make production business publication wait for logging. The corrected gated-save test passed 12 consecutive runs, then the complete core suite passed. No old environment result is counted.
- Logs are in `/workspace/shared/task079-metadata-evidence/`: `core-tests-final.log`, `core-list.log`, `clippy-final.log`, `rustdoc-final.log`, `owned-poll-repeat.log`. Earlier diagnostic/harness failures remain in `core-tests.log` and the initial focused logs rather than being reported as green.

Independent review of this new metadata scope is pending. Reclamation and later deletion/shutdown summaries remain a separate next slice.

## Owner reclamation and resource-release slice

- Branch `task/079-owner-reclamation-diagnostics`, worktree `/workspace/shared/minicore-host-task079-retirement`, fixed base `19e7bbba78f70be61732452d34fbfc717e564e15`. This is new work on the restored implementation, not recovered old bytes or transferred test results.
- Actual empty-memory retirement records Retiring then Retired. Explicit Agent deletion carries its first real Retiring transition out through the already-established return-record bridge; successful deletion records Retired only after its real work, files and resources have settled. A failed delete does not report retirement.
- Agent Session removal and Host Agent removal are direct-index facts, not claims that unrelated files/history were deleted. The Host snapshots the retired Agent's actual mode, so deleting a persistent Agent does not reclassify it as memory after its store is released. Batch reaping does not borrow an unrelated caller's operation.
- Host-driven Agent shutdown records its actual draining/stopping phase changes. Memory reclamation is distinct from persistent resource release. The latter logs the actual retained lifecycle, including Retiring after a partial failed delete, not an invented Stopped or clean deletion.
- Known close/delete/shutdown operations are borrowed into existing owner work. Unknown background-reaper operations remain absent. The watcher inherits the subscriber and uses a short per-Agent reaping span, never a Host-lifetime span. Records and all span callbacks remain outside outer admission and owner guards; no Session method or host/collaboration bridge was modified.
- Existing owner join and direct Agent-directory deletion failures preserve their original Fault and receive one primary owner ERROR. Forwarding a child/default-metadata failure does not add another ERROR. The actual filesystem refusal is tested; no synthetic private owner-state injection or claimed forced-panic coverage was added for the existing join-failure branches.
- No new owner state, recovery endpoint, implicit retry, resource quota, public observation source or logger-dependent business barrier. Runtime, TUI, SPEC, Cargo.lock, shared diagnostics, and configuration/resource/schema paths are unchanged.

Fresh validation, same pinned Rust/dependency environment and isolated target:

- `cargo test -p minicore-host-core --locked --offline`: **324 passed**, zero failed/ignored. `cargo test -p minicore-host-core --all-targets --locked --offline -- --list`: 324 distinct names. This is core-only, not a full-stage/workspace approval.
- Six new integration cases exercise synchronous memory retirement without an executor, a real Runtime still Running at close and joined by the asynchronous reaper, persistent versus memory shutdown, a real independent file refusing Agent directory deletion with its true partial outcome, successful persistent cascade with unchanged acceptance receipt, and reclamation with no subscriber/closed writer. One additional reentry case covers memory retirement and persistent shutdown through public owner reads.
- The first reaper test initially used an already completed Runtime, which legitimately retired synchronously and retained its known close operation. The fixture now keeps the real Runtime running on an explicitly pending model stream before close, exercising the intended asynchronous reaper without weakening its no-borrowed-operation assertion. This was a test-boundary correction, not a production failure or an old red/green claim.
- `cargo clippy -p minicore-host-core --lib --tests --locked --offline -- -D warnings`: passed. `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core --no-deps --locked --offline`: passed. `cargo fmt --all --check` and `git diff --check`: passed.
- Logs: `/workspace/shared/task079-retirement-evidence/core-tests-final.log`, `core-list-final.log`, `clippy-final.log`, `rustdoc-final.log`; the initial fixture failure remains in `focused.log`.
- Independent review and integrated-root checks remain pending. The metadata source commit is separately under TASK082 review; this reclamation slice is not covered by an earlier approval.

TASK053's subsequent Session metadata operation bridge has an exact three-line Agent caller patch at `/workspace/shared/task079-session-operation-bridge/agent-callers.patch`, SHA-256 `680218bba204f0703b151cb5be85c6ecc77413b67d0bdefaee744c3b5b51a839`. It was apply-checked against 19e7bbba and handed to TASK053 for one compilable combined Session/caller commit. This branch does not pre-apply those pending Session signatures or duplicate that work.
