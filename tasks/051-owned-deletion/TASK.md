# Task 051: Delete explicitly owned data

## Frozen baseline and scope

- Base: `accc5ddd120444cabe90c4756f5efa2bb047392c`; isolated topic `task051-owned-deletion`.
- SPEC v4.93 is unchanged: SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Runtime stays exactly 0.10.0 at `69faf9196991524b5549499d6208a1eaf567bf56`; Cargo.lock remains pinned.
- Worker requested configuration: gpt-6-astra / max. No independent effective-runtime attestation is claimed.
- No push, main, TUI, storage implementation or context/history/compaction implementation changes.

## Request and receipt freeze

`session.delete` has `operation_id` and `close` (default false). Default admission requires closed and settled. `close:true` explicitly authorizes normal close and required settlement, then deletion of only that Session's owned data.

`agent.delete` has `operation_id` and `scope` (default `empty`). `empty` requires stopped with no retained Sessions. `sessions` contains an explicit list of direct Session IDs plus `close` (default false). The set must exactly cover the retained children at the Agent admission boundary. Unknown/unavailable children and in-flight creations cannot be silently omitted. `close:true` permits the required Agent stop and normal close of those Sessions; it never grants directory-wide recursion or deletion of collaboration descendants owned elsewhere.

The direct parent owns accepted deletion work through caller timeout/disconnect. The control receipt forms at the acceptance boundary after admission is fenced and work registered, not after the entire deletion. Receipts remain immutable. Existing Agent get and Session state expose the current direct owner's typed deletion state, explicitly requested scope and partial outcomes/faults. Successful removal returns normal not-found through the former child query. Session deletion receipts belong to Agent; Agent deletion receipts belong to Host, subject to their real owner lifetimes. Old child-owned receipts are not copied to an archive.

Current deletion facts separate pending/not-attempted work, physical deletion, memory reclamation and failures. Failures may follow already committed close/deletion steps and do not imply rollback or that all files remain. This is narrow state for the two deletion methods, not a generic operation or recovery platform.

## Exact specification boundaries

- Section 3, Agent lifecycle/default entry (lines 235–245): retirement fences, durable default clearing, stopped/no-child default deletion, explicit cascade.
- Section 3, Session lifecycle (lines 267–295): close is distinct from delete; real execution, tools, interactions, saves, result handoff and root-scope cleanup precede closed/release.
- Section 4, storage ownership (lines 313–330 and 420–424): direct recorded ownership, no implicit deletion, no recursive expansion, no removal of parent results or independent artifacts.
- Section 6 (lines 743–745, 977): typed owner facts, accepted work survives wait loss, formed receipts remain immutable; operation_id is correlation only.
- Section 8 (lines 1367, 1388): canonical agent.delete and session.delete methods.

## Reference reuse

Read minicore-agent reference `f36a87366d8e2fe0a1c8216463887902e471c739`, `src/agent.rs::delete_session` and `src/store.rs::delete_session`. Its worker-settlement lesson is suitable; its rename/recursive-directory removal is not this Host's behavior baseline. Reuse Host's actual owner task/receipt/retirement boundaries and existing `SessionStore::delete_closed` / `AgentStore::delete_empty` instead.

## Implementation checkpoints

1. `e107293448fc19879bece75c3c896afa76f1bfd4` — `feat(protocol): define explicit owner deletion contracts`.
2. `8b566c33e0a315d3f6bdb98b2d8b03067897092a` — `feat(core): delete data through fenced direct owners`.
3. `ab06a2847505ebe6af2de778378ee0c714f25e8b` — `fix(core): settle deletion saves and concurrent owner cleanup`.
4. `af9132a32aaf3fa948b0250b87727d1d1ec61f2f` — `test(tools): preserve received results across scoped deletion`.
5. `b918790c63dc2916b1a8c0ba91b1a46fcf46b838` — `feat(transport): route canonical owned deletion controls`.

The code is frozen at the fifth checkpoint for independent review. This task record is updated separately. The integration owner applied the narrow shared Session declarations and AgentInfo/SessionStateData projections before they were committed here. PROJECT.md remains under that owner's coordination.

- Host retains Agent deletion work and its accepted receipt. Agent retains Session deletion work and its accepted receipt. Their existing shutdown/retirement barriers include these real tasks; no global task table or receipt archive is added.
- Agent admission freezes the exact registered child set and fences create/start/update/default/close/delete work. Session admission fences send/update/close/delete. Work accepted before a fence either settles through its existing owner or causes factual admission rejection.
- Normal close still performs root collaboration cleanup. Direct-owner reservations prevent duplicate cleanup, including a cascade whose root closes another listed memory child. Verified memory reclamation is distinguished from physical deletion and from a missing requested object.
- Persistent default clearing uses the actual metadata-save boundary without selecting another Session. Session deletion waits for real journal append acknowledgment, Runtime/tool/interaction/necessary-result cleanup, and resource release before the existing storage delete API is invoked.
- Failed or partially committed deletion stays visible on the surviving actual owner. A retained failed-delete storage locator can be released by normal owner retirement/shutdown; no automatic retry, rollback fiction or new recovery service is introduced.
- Typed, OS stdio and TLS-WSS routes call the same owners. This branch adds two routes to its 49-route baseline; the separately owned context route is not included. Routing counts do not imply complete product implementation.

## Validation at the frozen code checkpoint

All commands used Rust 1.90 with `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, `/workspace/shared/cargo/bin` prepended to PATH, `TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, and the isolated `CARGO_TARGET_DIR=/workspace/shared/minicore-host-owned-deletion-target`. Cargo.lock was preserved. No new dependencies were added.

Passed at `b918790c63dc2916b1a8c0ba91b1a46fcf46b838`:

```sh
git diff --check accc5dd..HEAD
cargo fmt --all -- --check
cargo clippy -p minicore-host-core -p minicore-host-protocol \
  -p minicore-host-transport -p minicore-host-tools \
  --all-targets --locked --offline -- -D warnings

cargo test -p minicore-host-protocol --test deletion \
  -p minicore-host-core --test task051_deletion --test task033_tool_cleanup \
  --test entity_retirement --test delegation_owners \
  --test delegation_acceptance_fences --test host_shutdown \
  -p minicore-host-tools --test host_integration --test scopes \
  -p minicore-host-transport --test deletion --test public_controls \
  --locked --offline
cargo test -p minicore-host-core --lib task051 --locked --offline

cargo test --workspace --all-targets --locked --offline -- --list
cargo test -p minicore-host-core -p minicore-host-protocol \
  -p minicore-host-transport -p minicore-host-tools --doc --locked --offline
RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core \
  -p minicore-host-protocol -p minicore-host-transport -p minicore-host-tools \
  --no-deps --locked --offline
```

- Focused integration command: **112 passed, 0 failed**, across 13 selected targets. Cargo applies each named test target to every selected package that has it; both protocol and transport public_controls and both core and tools delegation_acceptance_fences were included.
- Separate real Journal append/release gate unit case: **1 passed, 0 failed**, 65 filtered out. Together the final focused code gate executed **113 unique cases**.
- All **26 newly added cases** were covered: protocol deletion 4; core task051 deletion 10; existing real tool-cleanup fixture 2; Journal gate 1; actual Runtime/agent-tool integration 4; transport deletion 5.
- Workspace inventory compiled and listed **750 unique cases in 87 test binaries**, 0 benchmarks. This is an inventory, not a claim that all 750 were executed or passed.
- Four-package doctests: **1 passed, 0 failed**; other three packages had no doctests. Strict rustdoc and all-target Clippy passed with warnings denied.
- Runtime provenance and unchanged SPEC digest were rechecked. The transport tests use real operating-system pipes and local TLS-WSS connections; Runtime fixtures use the actual bound Runtime with named test models/gates, not live external providers. Destructive actions touch only test-created temporary data.

Logs are retained outside the repository under `/workspace/shared/task051-`: `final-clippy.log`, `final-focused.log`, `final-save-gate.log`, `workspace-list.log`, `doc-tests.log`, and `rustdoc.log`.

## Known aggregate boundary and remaining review

One development aggregate command, `cargo test -p minicore-host-core -p minicore-host-protocol -p minicore-host-transport -p minicore-host-tools --all-targets --locked --offline`, executed **382 passed and 2 failed** before Cargo stopped. The two failures are the baseline transport adapters assertions `receipt_lookup_faults_are_not_method_unavailable` and `rpc_protocol_errors_and_notifications_do_not_dispatch_messages`: they still classify routed `agent.start` as unsupported, whereas its missing required parameters return JSON-RPC `-32602`. Log: `/workspace/shared/task051-targeted-tests.log`.

The integration owner independently fixed those assertions at `1bf0969288e24f3f77c00ff742acd93280ef005e` and reported 12/12 focused adapter tests. That separate fix is deliberately not copied into this branch. The failed aggregate was not repeatedly rerun, and its earlier success count is not a final full-suite pass.

Storage/diagnostics lease corrections and the cold-history/context work are independently owned, excluded from this source, and require the final composition gate. No full workspace execution, live-provider test, TUI test, push or merge was performed here. Independent deletion review and integrated validation remain the next steps; reaching the canonical route count is not product completion.
