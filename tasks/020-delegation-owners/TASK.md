# Delegation owner capabilities

## Baseline and scope

- SPEC.md v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Fixed starting checkpoint: `da7c51e90324797c9b1be759a8bbb53469392df5`.
- This task owns core trusted Turn binding, registered result transfer, independent notifications, and root cleanup fences. The adapter owns scope membership and the six agent-tool actions. Core does not depend on host-tools or maintain a Host task index.
- TUI is unchanged. There is no remote push authorization for this repository.

## Implemented checkpoint

- Candidates obtain a non-deserializable PreparedTurnBinding before fallible resolve_for_turn. Only the Session configuration/lifecycle claim activates it. All internal Loops reuse the bound tools, and ordinary authority expires on cancellation/finalization.
- Internal acceptance uses the same Host admission lock but no external connection identity. Public capability methods expose only the required owner operations.
- A target registers its sink before accepting the task. Its own real Runtime input-history callback establishes task consumption, so an unrelated old Turn cannot complete newly queued work.
- Actual TurnResult/history/partials/input ranges transfer outside Session locks after required saves and queue settlement. An independent notification permit targets the original direct parent with busy/idle mode chosen under its receive lock. It cannot send a new task and does not depend on the original Turn, Load or connection.
- Root closure seals reception/dispatch, runs adapter cleanup independently, and joins actual member close/save/resource retirement. Only the root holds the cleanup hook and pending-handoff count; membership remains in the adapter.
- Exact newly created memory Sessions can return a narrow cleanup permit to the adapter's owned startup continuation. Ordinary empty-object creation and persistent partial creation are not rolled back.
- Existing ToolCall observations retain factual child/task associations without changing the original ToolResult.

## Validation

- Local Runtime used for the initial checkpoint: `/workspace/shared/minicore-runtime-unbounded`, fixed `cd59b6ecbad5ccfb67552660290992662253219b` (0.10.0).
- Validation uses a command-line Cargo patch and temporary uncommitted version/lock changes. No path patch, vendor tree, bootstrap, or local dependency SHA is committed or represented as remotely published.
- Environment: RUSTUP_HOME=/workspace/shared/rustup; CARGO_HOME=/workspace/shared/cargo; PATH=/workspace/shared/cargo/bin:$PATH; TMPDIR=/workspace/shared/minicore-host-tmp; independent target; debug=0; incremental=0.
- `cargo fmt --all` passed.
- `cargo clippy -p minicore-host-core --all-targets --offline --config <same patch> -- -D warnings` passed.
- Core all-target tests passed before the final four fixture extensions; all ten final delegation tests passed. Core `--all-targets -- --list` reports 134 unique test cases, including 10 delegation cases. Full workspace gates remain pending.
- `cargo test -p minicore-host-core --test delegation_owners --offline --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-unbounded"'`: 10 gate-ordered actual-owner tests passed, including real-file Tool fast-child handoff, shared consumed ranges, explicit unconsumed-input discard, and a dropped close waiter.
- Gates use channels/semaphores, not timing sleeps. Models are explicitly scripted fixtures, while Host/Agent/Session/Runtime and the file tool are real. No live-provider or TUI claim is made.

## Integration follow-up

- The core implementation and scoped-observation stage 2 are composed in this topic. The adapter and transport owners own their independent integration commits; this topic does not claim those worktrees' validation.
- The integration owner applies the published Runtime pin and reruns combined gates. This topic's final gate below deliberately records the exact local command-line patch instead of claiming a remote dependency run.

## Close and cancellation review corrections

- Preceding checkpoint: `6536da608f7e97a72a02221695112bf74fb33193`.
- Real task qualification includes its actual Session Turn binding, including cancellation before the first Runtime model request. Input ranges retain real consumed order and any actually bound but unconsumed cancellation tail; queued unrelated work still cannot qualify an old Turn.
- All memory Closed transitions use one gate for pending result transfer, scope cleanup, required saves and close faults. A blocking sink or failing resource preparation cannot publish Closed early.
- Public and internal close workers now share the actual memory-retirement tail. An internal close is Agent-owned before any wait; cancelling the permit wait does not discard accepted work.
- Original weak target retirement makes cleanup an actual completed no-op rather than a false NotFound. Scope/close OnceCells cache only success. Explicit `session.resume` may continue only an already registered failed scope cleanup even during Host stopping; it never schedules new business work or retries automatically.
- Focused delegation suite: 13/13 passed. Core all-target suite passed; core Clippy with `-D warnings` passed, using the same fixed local cd59b6e Runtime patch. Unique core suite increased from 134 to 137 cases; a final workspace `--list` is still pending.

## Atomic metadata candidates

- Added TurnCapability update_agent/update_session methods taking one expected revision and complete config/name or config/title candidate. Conflict does not reread a newer revision or overwrite newer metadata.
- Agent and Session owners validate before admission, then submit config and metadata atomically. Persistent candidates use the existing owned record-save/confirmation path; title is loaded and preserved on close.
- SessionData now has optional title with an absent-field default. Storage rejects blank titles and same-revision config/name/title changes. Existing records without a title retain their original meaning.
- Real Host/Runtime tests cover same-snapshot memory updates and stale-revision rejection; persistent tests read the confirmed actual files and reopen the Host to verify title/name/config values.
- Delegation tests: 15/15 passed. Storage all-target tests passed. Core and storage all-target Clippy with `-D warnings` passed. All used the documented fixed local Runtime cd59b6e command-line patch; manifests/lock remain uncommitted validation scaffolding.

## Coordinator integration checkpoint

The independent integration worktree `stage/observation-delegation` combines the
published-dependency Host foundation with scoped observation and these owner
changes; it does not modify the frozen initial Host dev publication.

Topic-to-stage mapping: connection bridges `1a87f53` -> `4a5ced6`, `0eeb57b` ->
`fcabe86`; observer `da7c51e` -> `0abd4de`, `49af8f8` -> `d626914`, `39a138e` ->
`32b6a86`; delegation `6536da6` -> `f9f8508`, `19738a4` -> `46c90e8`;
metadata `5e41795` -> `b3a7bae`. The only initial cherry-pick conflict was
`PROJECT.md`; the current handoff was retained and the new observation sections
were appended. No production conflict was resolved by dropping behavior.

Fixed `46c90e834ddb5f3e16a54b634964a1cf9a92175c` passed `cargo check
--workspace --all-targets --locked`, then the full `scripts/check.sh` gate:
whitespace, fmt, strict Clippy, **370 unique tests**, **3 doctests**, and strict
rustdoc. The unique count was independently listed. This pass precedes metadata
`b3a7bae`; it is not a pass for later changes or for the unfinished tools bridge.

The check used the actual local Runtime retry source
`3cf57e3e1947b0464bd8843ed0508bfddfb2639a` via explicit command-line Cargo patch,
with a temporary lockfile restored afterward. No path override is committed.
The Runtime publication is separately authorized and underway; the final
integrated gate must use its verified remote revision. Linux/Rust 1.90.0,
debug/incremental disabled, two build jobs and a separate target were used.
Evidence: `stage-46c90e8-check.log`, `stage-46c90e8-full-local-runtime.log`,
`stage-46c90e8-test-list.txt`, and the corresponding metadata JSON in the local
validation directory. An initial validation wrapper incorrectly passed
`--config` to the rustfmt subcommand and exited before tests; the corrected
wrapper leaves fmt unchanged and patches only dependency-consuming Cargo
commands. It is not counted as a passing gate or retained in the repository.

## Observation-stage capability bridge

- Locally replayed observation prerequisites `49af8f89d67d741d445329e6a66f620086d98ef9` and `39a138ec42ea1abddd3c33427e79032f1e3e0426`; the integration owner already has these, so they must not be replayed again there.
- Added TurnCapability::history_read as a narrow asynchronous wrapper over the real paged SessionHistorySnapshot owner path; no full-history substitute or released-body cache.
- Core cargo check passed with command-line patch `/workspace/shared/minicore-runtime-retry-state`, fixed `3cf57e3e1947b0464bd8843ed0508bfddfb2639a`. This is explicitly local validation, not a claim that this SHA is remotely available. Final scope tests below use this exact retry-capable runtime.

## Preparation-close handoff correction

- The production adapter's real close_scope integration exposed a missing transfer after a child was closed while resources were preparing. Inbox clearing correctly recorded the unconsumed task as discarded, but the preparation fence prevented immediate handoff and no preparation exit retriggered it.
- Driver termination and preparation-failure exits now attempt the existing handoff outside the Session lock after preparation settles. Closed/retirement still waits for actual transfer, required saves, and owned cleanup.
- Added two channel/semaphore-ordered Host/Runtime tests: successful and failed preparation after close both transfer only the discarded task fact, publish no Turn, and let the real close/retirement complete.
- Focused delegation suite: 17/17 passed using the exact local Runtime `3cf57e3e1947b0464bd8843ed0508bfddfb2639a` patch described above. The adapter's separate integration is rerun by its owner; no fixture is represented as that adapter result.


## Shutdown and retained-inbox verification

- Preceding preparation-close fix: `04fc9b8543aeab40613c4ee9416dd15da9ed2f98`.
- Added five gate-ordered actual Host/Runtime cases: draining rejects a new delegation to an existing Session but accepts a prior registered result; force joins actual partial-result handoff and cleanup without starting a notification Turn; persistent cancellation preserves a queued input and delays final handoff until its next real Turn and confirmed JSONL writes; a grandchild continues after its direct parent is reclaimed and never retargets the root; failed force cleanup runs once until an explicit resume continues only that cleanup.
- These tests subscribe before shutdown and wait for actual model starts, output acceptance, cleanup entry, and owner states. They do not use sleeps, observer streams as result delivery, or fake save acknowledgement. The persistent test reads the actual history file and checks the partial entry and both terminal records.
- Final topic gate at `04fc9b8` plus this test-only change: whitespace, fmt check, workspace all-target Clippy `-D warnings`, workspace all-target tests, workspace doctests, and rustdoc `-D warnings` all passed. Commands use `--locked --offline --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-retry-state"'` with exact local Runtime `3cf57e3e1947b0464bd8843ed0508bfddfb2639a`.
- Exact `cargo test --workspace --all-targets --locked --offline --config <same patch> -- --list` count: **354 unique test cases**, including **22 delegation cases**; **3 doctests** passed separately. The nested storage subprocess probe is not an additional case. This count is for this topic, not the parent's larger composed workspace.
- Validation logs: `/workspace/shared/delegation-final-gates.log` and `/workspace/shared/delegation-final-list.log`. Temporary validation manifests/lock are not committed. No TUI or live-provider validation and no remote push occurred.

## Integration test ordering correction

The combined `713efb0` full gate failed the existing
`dropping_scope_close_wait_does_not_drop_actual_owner_cleanup` test: it released
the cleanup gate before establishing its observation, allowing correct memory
reclamation to make that setup query return `AgentSessionNotFound`. The test
now registers its watch before releasing the gate. Caller-abandonment, actual
cleanup and original assertions are preserved; no production behavior changes.
The failed gate is retained as `stage-713efb0-full-published-runtime.log` and is
not counted as a pass. Independent review's two DSH P1 findings remain separate
work and are not claimed fixed by this test-only ordering correction.
