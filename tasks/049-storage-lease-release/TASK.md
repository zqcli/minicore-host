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

In progress. The fix must use a final-owner destructor around each successful
lock, including initialization/error exits. Root Directory clones and child
handles must share the actual guard, and journal cleanup must release its file
before its root owners. No save-range, byte, sync, validation, or error behavior
may be weakened. Full tests and independent review remain pending.
