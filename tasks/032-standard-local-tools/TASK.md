# Task 032: Standard local tools

## Scope and ownership

Implement the ordinary `read`, `edit`, `write`, and Unix `bash` Runtime tools in
`crates/host-local-tools`, independently of the `agent` tool. Baseline:
`04ba5bc07a7d9bb73eabb143839fbaeb264a0cea`; Runtime pinned to
`69faf9196991524b5549499d6208a1eaf567bf56` (0.10.0). Reference behavior was inspected
at minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739`; its old output quotas,
newline substitutions, and owner model were not copied.

The parent owns shared workspace manifests and integration. Bootstrap commits
are `1e8caab63f126c9254d50467fe153fdd1bb46686` and
`da1e354b283413a01e01ef143a19eaf5b312fded` (locally cherry-picked as `0198fb9` and
`691dfe4`). No changes to SPEC, TUI, Runtime, Workspace, or existing agent tools.

## Contracts implemented

- One frozen Workspace and one narrow, Turn-bound LocalToolSink per LocalTools.
  Model arguments cannot provide Session, Turn, or ToolCall identity.
- Explicit read ranges, exact literal edits, same-directory atomic writes,
  preserved existing permissions, real file snapshots for each targeted write.
- Scoped paths protect directories and Git metadata; write symlinks are rejected.
  Ordinary filesystem/matching errors are structured business results.
- Real `/bin/bash`, child-local cwd, exact complete stdout/stderr (binary output
  remains bytes), actual exit code/signal, unknown arbitrary shell file changes.
- Cancellation/deadline stops the Unix process group and actually waits for the
  direct child. Read pipes reach EOF or are explicitly incomplete after cleanup.
- Runtime 0.10.0 drops stopped port futures. A Turn-owned worker collection keeps
  execution alive and `finish()` closes admission and joins it, independently of
  the Runtime waiter or a dropped finish waiter. Internal Loop continuation does
  not close this collection. No side-effecting execution is automatically retried.
- Sink failure is an internal owner fault, distinct from command nonzero exit.
  Completion records retain actual outcomes for the integrating Turn finalizer.

## Validation

The tests explicitly label their scripted model and HTTP/SSE fixtures. Filesystem,
Unix subprocess, production provider adapter, and pinned Runtime execution are real.
The suite covers the normal Runtime file pipeline, production HTTP write/bash loop,
exact conflicts, missing/binary paths, symlink and directory protection, large UTF-8
reads/output, non-UTF-8 command output, real-pid cancellation/deadline cleanup,
abandoned waiters, no-observer completion, no side-effect retry, owner sink failure,
Runtime policy denial, internal Loop continuation, parallel same-binding edits,
and the same upstream ToolCallId in separate Turn-bound instances.

Initial focused suite: 20 tests passed. Three additional boundary regressions are
included for the final validation pass. Formatting and focused strict Clippy are
run before the implementation commit. Full workspace gates and unique case count
are recorded in the follow-up verification commit.

Environment: RUSTUP_HOME=/workspace/shared/rustup,
CARGO_HOME=/workspace/shared/cargo, PATH=/workspace/shared/cargo/bin:$PATH,
TMPDIR=/workspace/shared/minicore-host-tmp, CARGO_BUILD_JOBS=2,
CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0, CARGO_INCREMENTAL=0,
CARGO_TARGET_DIR=/workspace/shared/minicore-host-standard-tools-target.

## Explicit limits and next integration

This commit supplies a standalone execution module, not production Session
registry/persistence/finalization integration. The integrating owner must bind the
sink, select the actual ToolSet, retain LocalTools for the public Turn, and await
`finish()` before publishing finished/reclaiming resources. The sink is not an
observer; subscriber disconnection cannot fail authoritative records.

Unix bash only; no Windows process-group support, external hostile filesystem
races, cross-process write locks, detached-process sandbox, or apply_patch is
claimed. Directory creation is not an atomic directory transaction. Bash inherits
the Host process environment and is an execution capability, not an OS sandbox.
One second is a post-stop pipe cleanup allowance, not a content quota; incomplete
output is never presented as a clean EOF. Full stdout/stderr remains in the result.

## Cleanup settlement follow-up

Implementation commit: `43ed31618689c721c47eb4bf589ea3b9dff7d7ff`.
The first workspace pass listed 517 cases (494 baseline + 23 new). During integration
review, the core owner correctly required an explicit distinction between an
internal recording fault after completed cleanup and an unconfirmed execution.

`LocalToolCompletion.cleanup_confirmed` and `CommandResult.cleanup_confirmed` now
carry that distinction. A normal blocking file worker is joined even when its
record callback fails. A command is confirmed only after its real child wait,
nonfailed group stop, and completed owned-pipe cleanup. A panicked worker or failed
wait cannot manufacture confirmation. The core adapter must keep finalization
blocked for false, but can retain a diagnostic and finish for true.

Three additional scope regressions cover explicit worker panic (false), a joined
record error (true), and a dropped waiter retaining a real file worker with one
result. The focused suite now passes 26 cases and strict Clippy.

## Final verification

Verified source checkpoint:
`3cdd977c70e559286a72cf06e485a5e8a89b4261` (cleanup settlement follow-up).
All commands below completed successfully in the environment listed above:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --all-targets --locked`
- `cargo test --workspace --doc --locked`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`
- `cargo test --workspace --all-targets --locked -- --list`

The list contains 520 cases: 494 baseline cases and 26 local-tool cases. Nested
storage subprocess output is not counted as another test. The local-tool cases
comprise 3 ownership/settlement unit tests, 10 file tests, 1 production HTTP/SSE
fixture integration, 9 Unix process tests, and 3 real-Runtime boundary tests.
No checks were skipped for the supported Linux scope. Non-Unix execution and a
live remote model were not claimed or tested. The worktree was clean at the
verified source checkpoint before this documentation-only verification update.

The parent integration owner has the exact sink/finish contract and must complete
production registry selection, owner output/change persistence, and finalization
binding. Shared-core integration and its independent review are separate from
these module results. Public UTF-8 output projections must not silently decode
arbitrary raw command bytes with replacement characters; CommandResult retains
the exact original bytes and complete/incomplete facts.
