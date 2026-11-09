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

## Remaining

- Rebase the later scoped-observation retry/interaction commits after preserving this independent checkpoint.
- Add same-revision atomic metadata/config updates and persisted Session title.
- Expand force/draining, preserved inbox/partial persistence, intermediate-parent release, and actual adapter integration tests. Run final workspace lint/tests/docs and report the exact `--list` unique count; focused counts are not workspace totals.

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
