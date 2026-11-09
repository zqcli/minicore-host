# Remove the prompt-history fixture's inherited-lock race

## Scope and finding

Base Host: `34641c5ab1784237cb73bdb0f36f9923697d0c34`. Runtime remains actual Git
`69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0, without overrides.
SPEC v4.93 is unchanged. The parent owns the aggregate `PROJECT.md` update.

The final dev gate failed in task024's synchronous seed with `HistoryInUse`
after journal drop and `HostStorageUnavailable` after returning from seed.
The original binary reproduced a root-lock failure on run 39 of 39 normal
parallel runs. Its hash helper spawned `sha256sum`; that process could inherit
another parallel test's locked open file descriptions until close-on-exec.
Dropping the parent's handle therefore did not yet release the actual lock.

Direct diagnostic evidence captures both classes. A real hash child retained
the exact inode's `FLOCK WRITE`; the parent's original descriptor then closed
successfully, and its new descriptor's lock attempt returned `EAGAIN` before
that child's `posix_spawnp` returned. History evidence is in
`trace-v4-run-5.log` (child198 / inode363573), root evidence in
`trace-v4-run-12.log` (child247 / inode364115).

Evidence directory: `/workspace/shared/minicore-host-lock-reopen-logs`.
`DIAGNOSTICS.md` records commands, exact event order, run counts, diagnostic
source and limits. `strace` was blocked by `PTRACE_TRACEME = EPERM`; no settings
or permissions were changed to bypass it. Alternative observation uses libc
boundaries and only this test process's direct-child FD lock metadata, not a
kernel syscall trace. Logging can change scheduling; the uninstrumented red
run remains separate. The older storage-suite flake is not diagnosed here.

## Change and commits

- Shared manifest bootstrap `0fb766ed22be24177d1d06b4d665b4f1da21fae8`, locally
  `aea4712`, adds the existing locked `sha2 = 0.10.9` as a core dev-dependency.
  The parent already owns this commit and must not replay the local cherry-pick.
- Fix `d2edb50411045b805e86828cd72025b79e7bdf31` hashes the same bytes in process.
  All original SHA-256 comparisons, exact byte-prefix/native-history checks,
  actual Host/Runtime/storage/HTTP tests and normal concurrency are preserved.
  One known-answer test checks empty-input and `abc` standard SHA-256 vectors.
- No production lock, ownership, cleanup, API or dependency is changed. No
  sleep, application retry, suite serialization, early unlock or push is added.

## Validation

Exact environment: `env.sh` in the evidence directory; independent target
`/workspace/shared/minicore-host-lock-reopen-target`, Rust 1.90.0, jobs=2.

- `cargo test -p minicore-host-core --test task024_prompt_history --locked`:
  **12 passed** (`fixed-task024.log`).
- Corrected binary, normal default concurrency: **200 suites / 2400 case
  executions, zero failures** (`fixed-repeat-summary.log`). Repeating green
  runs is a stability check, not the cause proof. The same diagnostic observer
  also confirms no hash process is spawned (`fixed-trace.log`).
- `./scripts/check.sh` on `d2edb504`: **passed**, including full-history/staged/
  worktree whitespace, rustfmt, all-target Clippy with `-D warnings`, all
  workspace targets, **3 doctests**, and rustdoc with `-D warnings`
  (`full-gate.log`).
- `cargo test --workspace --all-targets --locked -- --list`: **495 unique
  target/test-name cases**, excluding repeated storage subprocess output
  (`test-list.log`, `test-count.log`).
- `cargo metadata --locked --offline --format-version 1` verifies the actual
  Runtime pin. SPEC SHA-256 remains
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

A following documentation-only commit records these completed checks. Parent
integration and independent review remain separate; this is not full-product
release acceptance. No remote action was attempted.
