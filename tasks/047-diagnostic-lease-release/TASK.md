# Task 047: Release the completed diagnostic writer's lease

## Scope and fixed inputs

- Base: `76a7bfbe7f70d7f707d48cc38190829d14c137f2`.
- Own only `crates/host-diagnostics` and this task record. No push is authorized.
- SPEC v4.93 remains unchanged (SHA-256
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`).
  Section 12 requires coordinated file output, application-owned shutdown,
  bounded waiting, real failures, and stdout isolation.
- Runtime remains published 0.10.0 at
  `69faf9196991524b5549499d6208a1eaf567bf56`. TUI, manifests, and Cargo.lock
  remain unchanged. Agent is a code reference, not a behavior baseline.

## Original failure and deterministic proof

The public-controls aggregate at `c4d3cbe14f0c88011057e57cd1b05d9f515de165`
failed unchanged `tests/file_lease.rs:50`: shutdown returned `Flushed`, then
preparing the hard-link alias returned `LockOutput(WouldBlock)`. The writer
already dropped its File before sending completion. Full original log:
`/workspace/shared/public-controls-final-check.log`.

At this task's fixed base, the unchanged parallel `file_lease` binary passed
74/100 runs and failed 26/100 at that same assertion. The suite also creates
real subprocesses. `/workspace/shared/diagnostic-lease-original-repeat.log`
retains the individual failures, not just the successful reruns.

A pipe-synchronized Python fork probe established Linux flock ownership:
closing the parent's descriptor does not release the lock while a live child
holds the inherited open-file description. Explicit LOCK_UN releases it while
the child remains alive. Both branches acquire after child exit. Probe log:
`/workspace/shared/diagnostic-lease-fork-proof.log`; reusable source is
`/workspace/shared/diagnostic-lease-fork-proof.py`. No tracing, ptrace, sleeps,
or filesystem permission changes were used.

The committed Linux regression preserves that inherited description in a real
child's stderr across exec, making the brief ordinary fork/exec window stable.
The child does not write or change the lock. The parent uses the same private
file acquisition and writer setup as public `prepare`, checks exclusive aliases,
shuts down, and attempts reopening while the child still holds its descriptor.
This deterministically fails on close-only teardown after `Flushed`. The child
is reaped before the regression assertion. Red log:
`/workspace/shared/diagnostic-lease-deterministic-red.log`.

The only setup refactor extracts the existing File lock/start path into a
private function, without changing behavior. It allows the regression to retain
an ordinary duplicate before handing the original File to its actual owner.
It is not a production injection hook or a mock lock.

## Implementation and focused verification

Red regression checkpoint: `fd23ff828c35a38f047568c80df9fc8c103cedb0`.
The worker now calls the file's explicit `File::unlock` after draining accepted
records and attempting its final flush, then drops the File before signaling
completion. Stderr has no file lease. The private release function receives only
the worker's own writer, not a shared service or a caller-owned lock.

The caller's bounded shutdown does not perform release. A real FIFO writer keeps
its lease after timeout until it actually drains and ends. The blocked-writer
unit test separately verifies that release was not called while writing was
blocked. Existing write/flush failures retain their actual counters and `Failed`
outcome, and do not skip release. An observed unlock failure has one private
health flag: `Failed` and `degraded`, without adding to write/drop counts or
printing raw errors. No retry, sleep, process-wide test serialization, separate
lock path, recovery framework, or new public API is introduced.

Focused checks pass 21 unique diagnostics cases and 1 doctest:

- `cargo test -p minicore-host-diagnostics --locked`
- `cargo clippy -p minicore-host-diagnostics --all-targets --locked -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Logs are `/workspace/shared/diagnostic-lease-green-focused.log` and
`/workspace/shared/diagnostic-lease-focused-clippy.log`. The inherited-descriptor
regression also checks real JSONL output and that child exit cannot unlock the
next writer's independent lease. The subprocess stdout-isolation test now
includes an injected release error.

## Final fixed-code verification

Implementation checkpoint: `8fc7db845c642507941c339eb67c83ae6f35a7a6`.
The final evidence commit changes only this task record.

Environment: `RUSTUP_HOME=/workspace/shared/rustup`,
`CARGO_HOME=/workspace/shared/cargo`, Cargo bin prepended to `PATH`,
`TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_BUILD_JOBS=2`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_INCREMENTAL=0`, and
`CARGO_TARGET_DIR=/workspace/shared/minicore-host-diagnostic-lease-release-target`.
The committed lockfile was retained throughout.

At the implementation checkpoint:

- The original parallel `file_lease` suite passed **500/500** repetitions,
  including the real process exclusion/reopen and FIFO timeout cases. This is
  the same suite that failed 26/100 before the fix, with no harness serialization.
- The deterministic live-child inherited-descriptor regression passed
  **100/100** repetitions. These are repeated executions, not extra test cases.
- `./scripts/check.sh` passed in full: historical/uncommitted whitespace,
  formatting, workspace all-target strict Clippy, workspace all-target tests,
  workspace doctests, and strict no-deps rustdoc.
- `cargo test --workspace --all-targets --locked -- --list` listed **652 unique
  unit/integration cases**. Nested child probe execution is not counted again.
  Workspace doctests passed **4** cases; diagnostics contributes 21 cases and
  one doctest to these totals.
- `git diff --check` passed. SPEC retains its original SHA-256, and no shared
  manifest, lockfile, Runtime, TUI, or non-diagnostics source changed.

Evidence logs:

- `/workspace/shared/diagnostic-lease-green-repeat.log`
- `/workspace/shared/diagnostic-lease-full-gate.log` (starts with fixed source SHA)
- `/workspace/shared/diagnostic-lease-test-list.log`

## Limits and next steps

Implementation and this task's full gate are complete. Independent review and
parent integration, including a new gate on the combined source, remain pending.
The parent owns the aggregate PROJECT record. Nothing was pushed.

The inherited-OFD proof/regression is Linux-specific; this run does not claim
Windows/macOS validation. Unlock failure behavior uses an explicitly injected
private release error, not a claimed real filesystem failure. Existing tests
exercise real processes, files, aliases, and FIFO blocking, but do not establish
live external-provider or TUI integration.
No storage correction is included. A separately reported storage HistoryInUse
failure uses a similar primitive but needs its own ownership proof and fix.
