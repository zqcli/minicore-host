# Task 039: Shared process diagnostics

## Scope and status

Implement the SPEC v4.93 chapter 12 process pipeline in `crates/host-diagnostics`:
application-owned setup, structured JSONL, early filtering, owned async identity,
single writer, minimal observed health, and bounded shutdown. Implementation and
focused tests are complete; independent review and final integrated workspace
gates remain pending. No remote action is authorized by this task.

This stage does not claim business event coverage, HostConfig application wiring,
`host.info` integration, executable lifecycle, or live provider/TUI validation.
No Runtime, core, protocol, transport, resources, or workspace behavior is changed.
No `logs.*` method, telemetry collector, quotas, rotation configuration, permission
change, emergency channel, or cross-process logging service is added.

## Fixed inputs and ownership

- Host base: `a5c4bf4451123d994bdd7cceb13787e94bbbcb99`.
- SPEC v4.93: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Agent code reference: `f36a87366d8e2fe0a1c8216463887902e471c739`; not a
  compatibility baseline. Reviewed CLI initialization and provider safe-metadata
  logging/redaction tests; differences and reuse are recorded in the crate README.
- Runtime remains exactly `0.10.0` at
  `69faf9196991524b5549499d6208a1eaf567bf56`. TUI unchanged.
- Parent-owned dependency bootstrap `cb719eda2f8c1c09bdecc63de2822c68eead41be`
  was cherry-picked as `20fa9a6`; shared manifest and lock updates are exclusively
  that bootstrap. This worker owns only crate contents and this task record.

## Design and dependency decision

`install`/`prepare` take application `Options`, returning one subscriber/exit
`Guard`. Hosts receive only cloneable read-only `Health`. `EventSpec` admits safe
static summaries and declared scalar attribute keys; `EventData` copies actual
protocol IDs and the existing Fault pair/diagnostic ID without its body/details.
`Context::instrument` retains owned snapshots per poll. Explicit child context
replaces the parent chain instead of accidentally inheriting a finished Turn.

Exact direct dependencies are tracing 0.1.44, tracing-subscriber 0.3.23 (only
registry/std), time 0.3.55 (formatting), serde 1.0.229, serde_json 1.0.154, and the
existing host-protocol crate. Tests use tempfile 3.27.0 and tokio 1.54.0. No
additional backend dependency was added.

Verified registry source of tracing-appender 0.2.5 still contains the stdout
`println!` timeout branch in `WorkerGuard::drop`, and a raw stderr flush-error
fallback in `worker.rs`. It is not adopted, wrapped, globally redirected, or
concealed by leaking its guard. A private std thread/channel writer instead has
no fallback output and reports actual write/flush failures. Timeout cannot
promise tail loss or successful flush, so health is degraded with unknown drops
omitted. Source formatting remains on the caller; output is asynchronous.

## Verification

Environment: `RUSTUP_HOME=/workspace/shared/rustup`,
`CARGO_HOME=/workspace/shared/cargo`, `CARGO_BUILD_JOBS=2`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`,
`CARGO_TARGET_DIR=/workspace/shared/minicore-host-diagnostics-target`.

Final focused gate passed 15 unique unit/integration tests plus 1 doctest:

- `cargo test -p minicore-host-diagnostics --locked`
- `cargo clippy -p minicore-host-diagnostics --all-targets --locked -- -D warnings`
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-diagnostics --no-deps --locked`

Additional coverage includes every shared identity field, unrecognized Fault
detail exclusion, all five levels, and append-only output. `cargo fmt --all --
--check` and `git diff --check` pass. `cargo test -p minicore-host-diagnostics
--all-targets --locked -- --list` lists exactly 15 unique cases. Full workspace
tests are running; final results and commits will be recorded before handoff.

## Honest limits and next steps

Tests model multiple Hosts with distinct real protocol IDs, not live Host owners:
4 independent contexts on 2 Tokio threads, 3 ordinary OS threads, real temporary
files, and injected output failures. Subprocess stdout assertions include the
fixed libtest startup banner and otherwise only the exact RPC probe line; normal
stderr parses as JSONL, failure/explicit-timeout/Drop-timeout branches are silent.

The channel is unbounded; a permanently stuck writer can retain queued serialized
records and its thread past the bounded exit wait. Failed I/O can leave partial
bytes; no retry or repair promises are made. Flush does not sync business data.
Sources must review declared scalar values for safety; arbitrary strings are not
automatically sanitized. No full owner/session/request objects cross the worker.

Next: independent review, final integrated workspace checks, then separate narrow
Host/Agent/Session/Turn business instrumentation, host.info health mapping, and
application startup/shutdown wiring. One Host's shutdown must never own this guard.
