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

Initial fixed-code checks pass: `cargo test -p minicore-host-storage --all-targets
--locked` and `cargo clippy -p minicore-host-storage --all-targets --locked -- -D
warnings`. Logs are `storage-green.log` and `storage-clippy.log` in the evidence
directory. Formatting and `git diff --check` pass. Repeated normal-concurrency
runs, full gate, and independent review are pending. `env.sh` fixes the Rust
1.90.0 environment and a dedicated target directory; dependencies stay locked.

The normal-cleanup fix does not claim to run Rust destructors after process
kill or power loss. Abnormal-exit lease release still requires the OS to close
all copies of the locked description. No recovery framework or timeout is added.
