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
preparation, and process signal/transport composition are being implemented.
Application subprocess acceptance and independent review are not yet complete.
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
