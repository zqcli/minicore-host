# Remove the prompt-history fixture's inherited-lock race

## Scope and fixed sources

- Base Host: `34641c5ab1784237cb73bdb0f36f9923697d0c34`, isolated branch
  `fix/history-lock-reopen`.
- Actual Runtime: Git `69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0,
  with no dependency source override. SPEC v4.93 remains unchanged.
- Shared manifest bootstrap `0fb766ed22be24177d1d06b4d665b4f1da21fae8` is locally
  cherry-picked as `aea4712`; it adds the already locked `sha2 = 0.10.9` as a
  core dev-dependency. The parent already owns this bootstrap.
- Only task024's hash fixture and this task record are changed here. Production
  file locks, storage ownership, Session cleanup, and public APIs are unchanged.
  No retry, sleep, serialization, explicit early unlock, or remote push is added.

## Reproduction and root cause

The final dev gate failed at task024's synchronous `seed`: dropping its journal
was followed by `HistoryInUse`, and returning from seed was followed by
`HostStorageUnavailable` while reopening the same data root. Distinct tests use
separate temporary directories; no Rust save task survives this seed.

The TCP/SSE fixture does not create a process, but the hash helper used
`Command::new("sha256sum").spawn()`. In a parallel suite this process creation
briefly inherits the open file descriptions of other tests, including their
exclusive `flock` locks, before close-on-exec runs. Dropping a parent's last
handle does not release a lock while that inherited description remains open.

Evidence is retained at `/workspace/shared/minicore-host-lock-reopen-logs`:

- Original gate: `../minicore-host-validation/dev-reviewed-final-full.log`.
- The unmodified original test binary reproduced the root-lock failure on run
  39 of 39 normal parallel repetitions (`baseline-run-39.log`). Earlier green
  repetitions are not treated as proof that the defect is absent.
- `strace` was unavailable: `PTRACE_TRACEME` returned `EPERM`, including the
  approved diagnostic execution route. No permissions or security settings
  were changed.
- An external, diagnostic-only libc interposer logs successful `close`, actual
  `flock` results, and the real `posix_spawnp` call. Its observer reads only
  direct children of this test process and their lock metadata. It adds no
  delay, retry, altered spawn arguments, or production code. This is libc
  boundary and `/proc` evidence, not a complete kernel syscall trace.
- `trace-v4-run-5.log`: child 198, created for `sha256sum`, has fd 30 with
  `FLOCK WRITE`, device 39 / inode 363573 at time 26607.668670382. Parent 169
  closes its fd 30 successfully at 26607.669158469; a new reader on the same
  inode fails `flock` with `EAGAIN` at 26607.670336337. That spawn returns only
  at 26607.672264774. The actual seed fails with `HistoryInUse` at line 443.
- `trace-v4-run-12.log` independently captures a root inode (device 39 / inode
  364115) retained by hash child 247, the parent's successful close, and a
  failed exclusive reopen before that child's spawn returns. The actual test
  fails with `HostStorageUnavailable`.

These traces establish the hash subprocess as the transient inherited owner
for both task024 failures. They do not diagnose the separately reported older
storage-suite failure, and this change does not claim to fix that observation.

## Minimal fix and regression

Hash the same bytes in process with `Sha256::digest`. All existing before/after
SHA-256 comparisons, exact byte-prefix checks, native history checks, real
Host/Runtime/storage behavior, HTTP/SSE calls, and ordinary test concurrency
remain in place. A small known-answer test covers the standard empty-input and
`abc` SHA-256 values. No new production dependency is added.

## Validation

Environment: pinned shared Rust/Cargo homes, `TMPDIR` set to the shared Host
fixture temp directory, jobs=2, debug info and incremental compilation disabled,
and independent `CARGO_TARGET_DIR=/workspace/shared/minicore-host-lock-reopen-target`.

- `cargo test -p minicore-host-core --test task024_prompt_history --locked`:
  **12 passed**, including the new digest regression (`fixed-task024.log`).
- Full whitespace/fmt/Clippy/workspace-test/doctest/rustdoc gate and repeated
  fixed parallel suite: pending at this checkpoint. Independent review and
  parent integration remain separate steps; no release-readiness claim is made.

The parent owns the aggregate `PROJECT.md` update and should cherry-pick this
fix's commits, not the already owned manifest bootstrap.
