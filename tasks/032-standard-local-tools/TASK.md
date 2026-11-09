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
