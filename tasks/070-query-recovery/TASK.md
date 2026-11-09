# Restore terminal query and lightweight inspection contracts

## Scope and provenance

This is a new implementation and validation after execution-environment loss,
not recovery of the original missing source bytes or a rerun of a missing probe.
Base: `7cc0bd2b399c3ea1320647e13d87bf57376934a5`, the recovered source-index13
checkpoint on top of published history. The original later commits titled
"retain original terminal Turn revision", "preserve completed query revisions
across reload", and "reuse retained terminal state in result queries" were not
recovered. Their identifiers are historical evidence, not this implementation.

The requested engineering configuration is Astra/max; effective runtime settings
are not independently attested. Worktree: `minicore-host-query-recovery`, branch
`task/070-query-recovery`. SPEC v4.93, fixed Runtime
`69faf9196991524b5549499d6208a1eaf567bf56`, Rust 1.90.0, all manifests, Cargo.lock,
TUI, native history, raw internal handoff, and remote refs remain unchanged.
No credentials or live provider are used.

## Implemented changes

- `b31fe675e4fe2b48b2244308809a694a006b689f`: Finalization advances a distinct Runner revision and retains its
  lightweight `TurnStateData` on the existing owner-local Turn observation.
  Finished event, get/list, result/wait, and subsequent subscription baselines
  use that same terminal state. Later Turn activity and unload/load cannot
  replace its original revision or Load identity. There is no new registry,
  native result wrapper, or current-state fallback for a finished Turn.
- Result/wait reuse `TurnStateData`, removing the redundant `TurnResultInfo`
  definition. Public detailed queries still return the original bound
  `SessionConfig`; state events/baselines stay configuration-free.
- `f4a9f8156255d5ff7af7efe639801d0df429e41f`: The six-action agent tool removes bound configuration from Turn
  inspect, including reads with output. Per SPEC, `include_config` remains
  allowed only for Agent/Session; no new Turn option or schema is added.
- `607f5204a28aaf680d6d0a4fa6fcfcce435fcbd6`: At the coordinator's request for task071, successful ToolStarted
  processing publishes the Session after the ToolCall state is Running. The
  task071 worker owns the resulting work-summary fields and focused tests.

No changes were needed in core `types.rs`, `session.rs`, `history.rs`, `host.rs`,
`lib.rs`, or resource/configuration owners. The parent owns aggregate project
notes, independent review, full-workspace validation, and publication.

## Deterministic regressions

New tests drive real Runtime through explicit gated/scripted model streams.
Transport tests compare real OS-pipe and in-process public methods, the actual
finished state/completion events, subscription baselines, subsequent Turns,
configuration updates, and persistent unload/load using real temporary files.
Agent-tool tests execute real Runtime tool calls for active and completed Turns,
including actual retained output. Synchronization uses model gates and owner
facts, not sleeps or fabricated history/events.

Against unchanged base production, the two new query tests failed with:

- Finished revision `28` was equal to its Finalizing predecessor `28`.
- After persistent unload/load, get reported revision `34` instead of the
  original terminal `29` for the same Turn and original Load.

Both new agent-tool tests failed because inspect exposed full bound configuration.
All four passed after the fixes. These numbers are the new run's observations,
not a claim to reproduce the original lost `30`/`31` probe.

## Validation

Evidence directory: `/workspace/shared/task070-query-validation/`.
Environment is recorded in `env.sh`: isolated target directory, Rust 1.90.0,
locked dependencies, jobs=2, debug info disabled and no incremental builds.

- `cargo test -p minicore-host-transport --test task070_query_recovery --locked -- --nocapture`
  failed as intended against base production (`red-query.log`) and passed 2/2
  after the query fix (`green-query.log`). An initial test-fixture type-inference
  compile error was corrected before the red run; its output is preserved in
  `query-fixture-compile-error.log`.
- `cargo test -p minicore-host-tools --test task070_inspect_recovery --locked -- --nocapture`
  failed as intended 0/2 (`red-inspect.log`), then passed 2/2 (`green-inspect.log`).
- `./scripts/check-whitespace.sh` and `cargo fmt --all -- --check`: passed.
- Final executable checkpoint: `607f5204a28aaf680d6d0a4fa6fcfcce435fcbd6`.
- `cargo clippy -p minicore-host-core -p minicore-host-tools -p minicore-host-transport --all-targets --locked -- -D warnings`:
  passed on final source (`final-clippy.log`).
- `cargo test -p minicore-host-core -p minicore-host-tools -p minicore-host-transport --all-targets --locked`:
  passed all 471 affected-package test cases (`affected-tests.log`). This includes
  the recovered task054 query tests and all four new regressions after the
  task071 ToolStarted hook. No test was removed or weakened.
- The same package selection with `--all-targets --locked -- --list` lists 471
  unique cases (`affected-test-list.log`). This is not a whole-workspace count.
- `cargo test -p minicore-host-core -p minicore-host-tools -p minicore-host-transport --doc --locked`:
  passed the one affected-package doctest (`doctests.log`).
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core -p minicore-host-tools -p minicore-host-transport --no-deps --locked`:
  passed (`docs.log`).
- Final whitespace/format checks and protected-file diff checks passed. SPEC
  SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`;
  Cargo.lock SHA-256 remains
  `151d9fb90f96072c9655f4f8a58ae4f4848f5caa2d1dc491fc2faa27ee2e79b8`.
- Full-workspace gate, independent review, TUI and live-provider validation have
  not been claimed by this task. The first two remain coordinator work; the
  latter two are outside this recovery task's scope.

## Follow-up: migrate remaining binary query fixtures

Base: `c872e1d4d039684fa124d8b5a28fab8efc4ab11c`, worktree
`minicore-host-query-fixtures`, branch `task/070-query-fixture-migration`.
Task067 identified an existing fixture that still treated public `turn.list`
as the pre-projection array. This migration addresses the same missing test
adaptation scope as original source step16; it does not claim to recover that
old commit's bytes.

Commit `a26498e28fd4984c7b7cdb7ca7bb8ed1847b2234` changes only existing tests:
`tool_support/mod.rs`, `tool_support/cleanup.rs`, `ordinary_tools.rs`, and
`process.rs`. Raw `turn.list` uses `data` and `turn_id`; raw `turn.wait` facts
use `data.info`. The restart-empty-list assertion checks the actual `data`
array. The already-correct typed result/wait paths remain untouched. There
were no raw `turn.get`, `turn.result`, or `message.get` fixture calls to adapt.

All behavior assertions, real process cleanup, subscriptions and teardown stay
intact. No test was removed, no process wait was bypassed, and no production,
manifest, lockfile or specification changed. Task067's new
`credential_environment.rs` is separately owned and is not included in this
base's existing-test count.

Evidence: `/workspace/shared/task070-query-validation/fixture-migration/`.
The same Rust 1.90.0, locked-dependency environment and task070-owned target
were used; exact environment is retained in its `env.sh`.

- Before edits, `cargo test -p minicore-host --test ordinary_tools binary_tools_produce_exact_files_native_results_and_canonical_sources --locked -- --exact --nocapture`
  reproduced a real binary-fixture failure at `tool_support/mod.rs:263`:
  top-level `as_array().unwrap()` saw the correct query object, not an array
  (`red-only-turn.log`). This was a stale test shape, not a production failure.
- `cargo test -p minicore-host --all-targets --locked`: passed all 17 existing
  cases after migration (`all-host-app-tests-attempt1.log`), comprising four
  ordinary-tool/cleanup tests and thirteen process tests. No separate behavior
  failure occurred after the mechanical shape correction.
- `cargo test -p minicore-host --all-targets --locked -- --list`: confirmed
  17 unique cases (`host-app-test-list.log`).
- `cargo clippy -p minicore-host --all-targets --locked -- -D warnings`: passed
  (`host-app-clippy.log`). Whitespace, formatting, `git diff --check`, and
  protected-source diff checks passed.
- Whole-workspace validation, independent review, and publication remain with
  the coordinator. No remote write was performed by this worker.
