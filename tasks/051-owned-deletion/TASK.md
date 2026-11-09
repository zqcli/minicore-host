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

The fifth checkpoint was the initial independent-review source. The acceptance-fence correction below supersedes that code checkpoint. This task record is updated separately. The integration owner applied the narrow shared Session declarations and AgentInfo/SessionStateData projections before they were committed here. PROJECT.md remains under that owner's coordination.

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

## Independent-review acceptance-fence correction

The independent reviewer found a real admission defect in `b918790`: Session deletion returned Closing before expiring its bound Turn capability or sealing its root collaboration scope, and Agent deletion returned Retiring before its async worker expired owned Turn capabilities. On current-thread Tokio, an immediate capability send or descendant registration could therefore accept new work after the deletion receipt, without any await in the gap. The earlier focused green suite did not cover this boundary.

The original reviewer probes and red run remain unchanged at `/workspace/shared/minicore-host-review-owned-deletion/crates/host-core/tests/task051_review_probe.rs` and `/workspace/shared/minicore-host-review-owned-deletion-evidence/review-probes.log`. SHA-256 of that original red log: `40831c208f0540dc92ac54d5ddc34c30fea52b2e317223066487a38179911e4e`.

Incremental commits:

- `1f50a25c401fca37082c7f20f446579f5e264ed0` — `test(core): expose deletion acceptance capability gaps`. Adds four permanent current-thread regressions in `crates/host-core/tests/task051_deletion_fences.rs`: Session source capability expiry; Session-root descendant scope admission; Agent-root descendant scope admission; and capability expiry for every active directly owned Session in an Agent cascade. All four fail on the unchanged production source, demonstrating actual MessageAccepted outcomes.
- `fd6051c398732b00f0aed015550078fa94266e2d` — `fix(core): fence deletion capabilities before acceptance returns`. Production changes are confined to the existing Agent and Session deletion modules: 31 lines added, 3 removed. The Session admission helper uses the existing Turn activation expiry and ScopeState seal, with Closing, before the receipt is formed. Agent fixes its direct child set, releases AgentState, then applies this same fence to each child under the shared admission boundary. Runtime cancellation, scope callbacks, joins, result handoff and saves remain existing asynchronously owned work. No callback, I/O or child call was added under an owner state lock; no cleanup retry or recovery service was added.

Red/green command:

```sh
cargo test -p minicore-host-core --test task051_deletion_fences --locked --offline
```

It produced **0 passed, 4 failed** at `1f50a25`, then **4 passed, 0 failed** at `fd6051c`. Logs: `/workspace/shared/task051-deletion-fences-red.log` and `/workspace/shared/task051-deletion-fences-green.log`. The fixture uses real persistent Host/Session/Runtime owners and gated test models. Its external scope hooks are an explicit no-op test double; each actual delegated member is settled through its original result/permit owner before controlled Host shutdown. Existing real host-tools scope-cleanup coverage is also rerun below.

Verification at exact fixed code `fd6051c398732b00f0aed015550078fa94266e2d`, using the same isolated environment and locked dependencies recorded above:

```sh
cargo test -p minicore-host-protocol --test deletion \
  -p minicore-host-core --test task051_deletion --test task051_deletion_fences \
  --test task033_tool_cleanup --test entity_retirement --test delegation_owners \
  --test delegation_acceptance_fences --test host_shutdown \
  -p minicore-host-tools --test host_integration --test scopes \
  -p minicore-host-transport --test deletion --test public_controls \
  --locked --offline
cargo test -p minicore-host-core --lib task051 --locked --offline
```

The integration command passed **116 cases**, and the real Journal append/release gate passed **1**, for **117 unique executed cases**. All **30 newly added deletion cases** are covered. Formatting, `git diff --check accc5dd..HEAD`, the same four-package all-target Clippy with `-D warnings`, the same **1 doctest**, and strict rustdoc passed again. The unchanged earlier adapter aggregate failures were not rerun or hidden by this correction. Logs: `/workspace/shared/task051-fence-final-focused.log`, `task051-fence-final-save-gate.log`, `task051-fence-final-clippy.log`, `task051-fence-doc-tests.log`, and `task051-fence-rustdoc.log`.

The workspace inventory at that intermediate checkpoint compiled and listed **754 cases in 88 binaries**. This intermediate green gate did not complete review: the reviewer next exposed the unbound-input race below.

## Normal-close disposition correction

The reviewer's next current-thread probe accepted a delegated input into a real memory Session, then synchronously accepted deletion before its queued driver ran. Both `b918790` and `fd6051c` could publish memory Closed before disposing that input; the async delete worker then skipped normal close's inbox clearing because the Session was already Closed. The actual snapshot retained `next_turn` with deletion Pending, and no discarded-input handoff arrived. The matching normal `session.close` probe passed. Agent cascade had the same defect. No save retry or recovery behavior was involved.

Original evidence is preserved in `/workspace/shared/minicore-host-review-owned-deletion-evidence/original-unbound-probe.log`, `fixed-unbound-probe.log`, and `fixed-unbound-both-probe.log`. The reviewer's disposable probe is `crates/host-core/tests/task051_review_unbound_probe.rs` in its original and fixed review worktrees. SHA-256 of `original-unbound-probe.log`: `42c9280ab544c725b4856cb373b1a96d3cdcac2f8b7a200fdf13dab0f85b4bd9`.

- `7dab5c96388653a25bdb506670499f90cdc1c790` — `test(core): expose unbound memory deletion handoff race`. Adds permanent real-memory Session and Agent variants to `task051_deletion_fences.rs`. Both fail before the fix with the same retained Closed/inbox/Pending state.
- `4b379623faa2398a00efe8a456d7c6085c65ffc3` — `fix(core): admit deletion through normal close disposition`. Replaces the partial Closing fence with the existing `start_scope_cleanup_locked` and `apply_close_locked` admission semantics. Inbox disposition, scope sealing/owned cleanup registration, and cancellation claims occur synchronously before receipt formation. Runtime handle shutdown and necessary result handoff remain asynchronous. Cascade child deletion reuses its already-admitted close instead of starting scope cleanup again, so one accepted deletion does not retry a completed failed cleanup. No additional state fields, callbacks under locks, storage changes, or recovery service are introduced.

Red command at `7dab5c9`:

```sh
cargo test -p minicore-host-core --test task051_deletion_fences unbound --locked --offline
```

Result: **0 passed, 2 failed**, 4 filtered out. Log: `/workspace/shared/task051-unbound-fences-red.log`. At `4b37962`, the complete `task051_deletion_fences` target passes **6/6**; log: `/workspace/shared/task051-unbound-fences-green.log`. The two added cases verify the actual discarded-input handoff, no fabricated Runtime result, no child Runtime binding, final notification through the original permit, one handoff, source reclamation, and controlled Host shutdown.

At exact final code `4b379623faa2398a00efe8a456d7c6085c65ffc3`, the expanded focused command printed in the preceding section passed **118 cases**, and the same real Journal gate passed **1**, for **119 unique executed cases** covering all **32 new deletion cases**. The same formatting/diff checks, four-package all-target Clippy with warnings denied, **1 doctest**, and strict rustdoc passed. Logs use the `/workspace/shared/task051-close-` prefix: `final-focused.log`, `final-save-gate.log`, `final-clippy.log`, `doc-tests.log`, and `rustdoc.log`.

The independent reviewer reported its unchanged probes green at this exact final code: original acceptance plus ordinary controls **6/6**, Session/Agent unbound-memory plus normal-close control **3/3**, and actual-process failed-save/force-shutdown **1/1**. It also confirmed that the normal-close/cascade reuse added no I/O, joins or child waits under owner state locks. Those independent probes are separate evidence, not additional repository inventory cases or a composition gate. Final independent broad checks and integration remain owned by their reviewers.

The final `cargo test --workspace --all-targets --locked --offline -- --list` command compiled and listed **756 unique cases in 88 binaries**, 0 benchmarks, at `4b37962`; log: `/workspace/shared/task051-close-workspace-list.log`. This is not a full workspace execution claim. SPEC, Cargo.lock, Runtime revision, storage implementation and TUI remain unchanged; no push or merge was performed. The only correction paths after `cb5c82c` are the two deletion modules, their new fence regression target and this task record.

## Reviewed deletion composition and full gate

The eleven task051 source commits through documentation-only `cf0e9e17187b7eada2cf367635dab2d024b7eef9` are integrated without conflicts at `4187899ef9b7301ed5af4cf36b84b5825fb83bed`. Independent review approved the fixed deletion scope after replaying both original P1 failures and ten unchanged probes. Its broader 488-pass/two-stale-adapter result is retained as such; the already-integrated adapter correction resolves those old expectations.

At the combined source, `./scripts/check.sh` passes the normal whole-workspace gate, **811 unique tests and 4 doctests**. `cargo test --workspace --all-targets --locked -- --list` independently confirms 811 target/name pairs across 88 nonempty targets. `cargo metadata --format-version 1 --locked` confirms the sole Runtime `69faf9196991524b5549499d6208a1eaf567bf56`; SPEC hash is unchanged and `git diff --check main..HEAD` passes. Exact log/metadata hashes are in `docs/executable-deletion-verification.json`; logs are `/workspace/shared/minicore-host-validation/executable-deletion-4187899-full.log`, `executable-4187899-test-list.txt` and `executable-4187899-metadata.json`. Earlier failed probes and aggregate records are preserved.

All 52 canonical methods now have shared real routes. Public query contracts, input provenance, complete business diagnostics and final performance/external review remain unfinished. No new remote publication, main merge, TUI edit or dependency change occurred.
