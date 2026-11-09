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
