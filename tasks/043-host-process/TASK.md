# Task 043: Assemble the Host process

## Scope

Build the local `minicore-host` executable on the real Host, production resource
preparer, stdio and authenticated WSS/TLS adapters, and process diagnostics.
The immutable design is SPEC v4.93. `minicore-agent` at
`f36a87366d8e2fe0a1c8216463887902e471c739` is a source reference only, not a behavior
baseline. Its small CLI/runtime ownership and parked-stdin shutdown handling are
reused conceptually; Host configuration, operations and lifecycle remain canonical.

Base: `4572488edf053b4f9d7e5f0c62225327506b0d97`. Workspace bootstrap:
`7631b994d10fc671bfb0ce4998dea5675233d4bd` (local cherry-pick `f002f8b`).
No dependency versions were changed. No TUI edits, live-provider calls or pushes.

## Trusted lifecycle capability

`Host::process_control` returns a non-serializable Weak-bound capability for
snapshot, cancellation-safe changed waits and explicit application shutdown.
It registers no connection and claims no external operation right. External
`Host::shutdown` is still checked against the actual connection's current rights.
Both routes share the same Host admission boundary and shutdown implementation.
The process notification is separate from the business wakeup, so merely
observing a cleanup fault does not retry cleanup. The handle does not hold an
otherwise abandoned idle Host alive; an active wait temporarily retains its owner.

Four focused tests cover actual first-client rights, the weak idle lifetime,
cancelled waits, repeated graceful with paused backlog, explicit force, original
failed creation saves, and real root-lock release only after stopped cleanup.
The existing shutdown regressions are also run. No new recovery path is added.

## Progress

CLI/config parsing, effective-config diagnostics installation, explicit PEM TLS
preparation, and process signal/transport composition are implemented. Eleven
actual-executable acceptance tests and targeted strict lint pass. Independent
review and final fixed-checkpoint workspace validation are still pending.
Production ordinary-tool assembly is a separate task and must integrate before
claiming executable tool acceptance. Business owner instrumentation and host.info
logging health are not provided by the process lifecycle log events.

## Initial validation

Using the configured Rust environment and a separate app target directory:

- `cargo check -p minicore-host --locked`: passed on the in-progress app tree.
- `cargo test -p minicore-host-core --test host_shutdown --test process_control --locked`: passed.
- `cargo clippy -p minicore-host-core --lib --test process_control --locked -- -D warnings`: passed.
- `cargo fmt --all`: applied; final immutable validation is pending the completed app.

Reports are retained outside the repository under the shared workspace with
`minicore-host-process-hook-` and `minicore-host-app-` prefixes. Final source
checkpoints, exact test totals and remaining limits will be recorded below.

## Actual diagnostic file ownership

Process integration exposed the need to coordinate configured file outputs
across different data roots. The real diagnostics File now acquires an exclusive
advisory lock before becoming the writer. File aliases share that lease. The
writer drops the File before acknowledging completion; a bounded flush timeout
does not release a File still owned by the blocked writer. There is no separate
application-side lease, output fallback, retry, or business durability claim.

The focused diagnostics gate passed all 19 unique unit/integration cases,
including four new cases for same-process/path-alias ownership, a nested real
subprocess, and a real FIFO writer blocked beyond the shutdown budget. The nested
probe is not counted again. `cargo clippy -p minicore-host-diagnostics --all-targets
--locked -- -D warnings` also passed. The original diagnostics pipeline review is
not claimed to cover this new change; independent review remains pending.


## Application implementation

- Trusted process hook: `8bff066118e0ac7615219583c962cefbfca9832a`.
- Actual diagnostic file ownership: `af0f88eff812239fb0152e78cba78a02420e9f37`.
- Binary/package: `minicore-host`; implementation directory: `crates/host-app`.
- `--data-root` is mandatory. `--config` supplies a new root's initial canonical
  HostConfig; saved config wins on reopen. `--stdio` and configured WSS may
  coexist. WSS requires explicit PEM certificate/key files and the configured
  access-key environment source. It never falls back to plaintext/anonymous.
- Diagnostics is installed once from the effective saved configuration after
  acquiring the root, before any executable owner or actual connection.
- SIGINT/SIGTERM request graceful. Repeated signals do not force. EOF or broken
  stdio closes only that connection; accepted work and WSS remain alive. A fatal
  listener/task failure requests graceful cleanup. Only explicit RPC force can
  upgrade application shutdown. Original save faults keep stopping/root ownership.
- Actual stopped cleanup precedes transport cancellation and runtime exit;
  only afterwards does the process request bounded best-effort diagnostic flush.
  A parked Tokio stdin reader does not keep a truly stopped process alive.
- Help, invalid inputs, TLS/configuration errors and panic summaries never use
  stdout or print source bodies, raw failures, credentials, paths or prompts.

The 11 subprocess tests use OS pipes/signals, real temporary storage, production
provider/resources/Runtime, a gated loopback HTTP/SSE fixture, and generated TLS
certificates. They cover normal startup and failures; saved identity/new
incarnation/root conflict; real message/output/history; draining accepted work;
WSS trust/hostname/authentication/operation rights; clean and broken stdio
connection lifetime; repeated signals versus explicit force; retained failed
save and externally killed abnormal exit; and log-file ownership across roots.

Initial app gates: `cargo test -p minicore-host --all-targets --locked` (11 tests)
and `cargo clippy -p minicore-host --all-targets --locked -- -D warnings` passed.
Only Linux execution is verified. No external model, remote-device or TUI
acceptance is claimed. Ordinary tools await the coordinated production binding.


## Fixed application checkpoint and storage-path correction

At fixed source `4e85b03be73c921d93edb3f3351da04967d0715e`, all of these passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --all-targets --locked`
- `cargo test --workspace --doc --locked`: 4 doctests.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`
- `cargo test --workspace --all-targets --locked -- --list`: 656 unique cases.
- Clean-tree and unchanged SPEC SHA-256 checks.

The diagnostics increment `af0f88e` separately passed independent review: 19
cases, 1 doctest, strict lint/docs/fmt and external process/alias/FIFO probes.
That review did not cover the application or process capability.

A subsequent small app-only startup check resolves the actual root and log
path, rejecting HostStore's fixed `host.json` metadata and `agents` storage
subtree before output opens. It permits an explicitly created `root/logs`
directory. Existing ordinary symlinks are resolved; a dangling/otherwise
unresolvable output link fails without fallback. It does not scan external
hardlinks or attempt to protect against concurrent malicious path replacement.
No diagnostics backend, storage ownership, configuration schema or new path
policy framework was introduced.

Two real executable regressions preserve the exact pre-start Host metadata and
Session history bytes for direct/aliased conflicting targets and confirm a
separate log directory inside the root works. The initial build exposed one
missed call-site argument and was corrected before testing. All 13 app cases and
strict all-target Clippy then passed. Temporarily removing only the path check
makes the original regression fail because the bad process incorrectly starts;
restoring it returns the same test to green. The deliberate failing run is
retained separately, not included as a passing gate.
