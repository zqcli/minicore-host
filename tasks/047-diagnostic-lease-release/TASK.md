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

## Status and next steps

Focused checks are green. Run repetitions and the full fixed-code workspace
gate, then obtain independent review before parent integration.
No storage correction is included. A separately reported storage HistoryInUse
failure uses a similar primitive but needs its own ownership proof and fix.
