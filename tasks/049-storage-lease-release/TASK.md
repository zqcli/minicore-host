# Release storage leases at final actual owner exit

## Scope and constraints

Base Host: `76a7bfbe7f70d7f707d48cc38190829d14c137f2`. This topic owns only
`crates/host-storage` and this task record. Parent integration owns `PROJECT.md`.
Runtime remains 0.10.0 at `69faf9196991524b5549499d6208a1eaf567bf56`.
SPEC v4.93 and TUI are unchanged. No remote action is authorized by this task.

SPEC section 4 requires exclusive actual-root ownership through all related
execution, necessary writes, synchronization, and cleanup. Journal exclusivity,
reader lifetime, and durable save confirmation are distinct. Caller departure,
save confirmation, or an unrelated forked child must not redefine the actual
storage owner's lifetime.

## Independent cause proof

A pipe-synchronized Linux fork proof covers the root directory, exclusive
history writer, and shared history reader. Closing only the actual parent's
file leaves its `flock` on the shared open-file description while an unrelated
child retains that description. Explicitly unlocking at final-owner completion
permits immediate acquisition while the child is still alive. The child never
reads, writes, unlocks, or accepts storage work. Its later exit cannot release a
new independent owner's lock.

The Rust regressions duplicate the actual acquired production descriptor into
a child process's unused stderr, preserving the same open-file description
across exec to make the fork-before-exec window deterministic. Pipe handshakes
replace timing guesses. No unsafe code, sleeps, lock retries, suite
serialization, strace, ptrace, or permission changes are used.

Before any production fix, seven regressions fail: final root owner, root
initialization sync error, journal writer completion, shared reader completion,
shared reader validation error, writer validation error, and deletion cleanup.
The accepted-worker lifetime control passes: dropping all callers does not
release the root or history while accepted work is still owned by its worker.

Evidence directory: `/workspace/shared/minicore-host-storage-lease-artifacts`.
`fork-proof.py` and `fork-proof.log` record the kernel ownership experiment;
`deterministic-red-all.log` records seven failures plus four passing controls
under normal test concurrency. Production acquisition/cleanup was unchanged in
this run; only test hooks and regressions were added.

## Implementation and verification

- Red regression commit: `9fc1fb2ba625c94d91d18ae7f85fe92f896363d3`.
- The private `FileHandle` owns an acquired lease and unlocks immediately before
  its File is closed. Unlocked child directories never unlock. Root Directory
  clones share `Arc<FileHandle>` rather than running per-clone cleanup or
  inspecting reference counts. The guard is installed immediately after a
  successful acquisition, before any fallible initialization work.
- Persistent journals retain the guard through accepted writes, failed-sync
  reconciliation, and save confirmation. Their fields release the history
  descriptor before directory/root owners. Temporary readers and deletion
  operations use the same guard through both success and error returns.
- No bytes, range positions, validation, synchronization, acquisition policy,
  business ownership, or public API change. Unlock is not a save confirmation.
- Additional controls check independent concurrent readers, a moved accepted
  worker after caller drop, and deletion failure preserving original bytes.

## Completed verification

Fixed source commit: `9dd9f7c251506ceeeb1cabebde56a2f58682ec67`. A following
record-only commit captures these results. The worktree remains isolated on
`fix/storage-lease-release`; parent integration and independent review are
separate acceptance steps.

All checks use the evidence directory's `env.sh`, Rust 1.90.0, locked
dependencies, jobs=2, debug=0, incremental=0, and the dedicated target
`/workspace/shared/minicore-host-storage-lease-release-target`.

- `cargo test -p minicore-host-storage --all-targets --locked`: **87 unique
  storage cases passed** (36 unit and 51 integration), normal concurrency.
  Log: `storage-green.log`.
- `cargo clippy -p minicore-host-storage --all-targets --locked -- -D warnings`:
  **passed**. Log: `storage-clippy.log`.
- Direct final unit binary, normal default concurrency: **100/100 suites passed,
  3,600 case executions**, no failure. Log: `unit-repeated-9dd9f7c.log`.
- Direct complete storage integration binary, normal default concurrency:
  **50/50 suites passed, 2,550 case executions**, no failure. Log:
  `integration-repeated-9dd9f7c.log`; summary: `repeat-summary-9dd9f7c.log`.
  Nested child probe invocations are not extra cases. These repetitions check
  stability; the synchronized red/green experiments establish the cause.
- `./scripts/check.sh`: **passed**, exit 0. Includes full-history/staged/worktree
  whitespace, rustfmt, workspace all-target Clippy with `-D warnings`, workspace
  all-target tests, doctests, and rustdoc with `-D warnings`. Log:
  `full-gate-9dd9f7c.log`. The independently tracked diagnostics failure did not
  occur in this gate run; diagnostics code is untouched by this topic.
- `cargo test --workspace --all-targets --locked -- --list`: **661 unique
  workspace target/test-name cases**, including 87 storage cases. Logs:
  `test-list-9dd9f7c.log` and `test-count-9dd9f7c.log`.
- `cargo metadata --locked --offline --format-version 1` confirms Runtime
  0.10.0 at the exact `69faf9196991524b5549499d6208a1eaf567bf56` revision and
  retained locked tokio 1.54.0. Artifact: `metadata-9dd9f7c.json`.
- SPEC SHA-256 remains
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
  No dependency, shared manifest, core, diagnostics, TUI, or remote change.

No verification blocker remains in this topic. No live-provider, power-loss,
network-filesystem, other-platform, or TUI acceptance claim is made.

The normal-cleanup fix does not claim to run Rust destructors after process
kill or power loss. Abnormal-exit lease release still requires the OS to close
all copies of the locked description. No recovery framework or timeout is added.
