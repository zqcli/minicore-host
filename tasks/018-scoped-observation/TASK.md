# Scoped observation and source-owned reads

## Scope and fixed inputs

Implement v4.93 chapter 9 through real Host/Agent/Session owners: source-owned live output, fixed-end UTF-8 range reads, Runtime stream draining, real history handoff completion, and connection-scoped dynamic subscriptions. TUI remains unchanged. No remote writes are authorized or attempted.

Design SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Host baseline: `8079970d3d7b35912ae4b45118d8e980d5f3e749`.
Runtime local validation target: `cd59b6ecbad5ccfb67552660290992662253219b` (0.10, unpublished). Published manifest pin remains `18a6ac9b82d003330cd870275efca793cdcc9a96` until the parent integrates dependency changes. Temporary command-line patch and uncommitted version/lock changes are validation-only.

## Ownership

The shared observation facility owns subscriptions and non-awaiting outbound queues only. It holds no Session/Runner or authoritative body cache. Session owns original model/tool output and obtains completed records through the existing Runtime history callback. Public events never reconstruct authoritative durable history. Scope filters follow actual ownership, not delegation. Registration precedes per-owner state/watermark capture; no global owner freeze. Dropped, revoked and explicitly ended subscriptions cannot affect execution.

## Progress

- Runtime mechanical migration and reserved Host network module integrated as local prerequisites.
- Protocol observation/range DTOs and typed lookup faults in progress.
- Runtime at this fixed commit does not expose authoritative model retry progress. This is a verified upstream capability gap, not silently synthesized from public events.

## Validation

Pending implementation and gates. Scripted gated model/tool fixtures will be identified as fixtures; no live-model/TUI claim.

## First executable checkpoint

Implemented dynamic scoped dispatch, exact connection ownership and cleanup,
initial state/watermarks, known-delivery-failure resync control, model/tool range
reads, actual Runtime drain and complete history-handoff events. Tool process
text uses a private binding and the actual invocation; it is not reconstructed
from ToolFinished summaries or final results. Source retirement clears its
retained output; subscription handles hold no Session/Runner. Six new real
Runtime integration cases and three protocol cases pass. Existing full workspace
tests passed before the final allocation-only notification boxing correction;
final gates and the unique list count are being rerun below.

The `bind_tool_output` hook is infallible and non-I/O and only wraps prepared
tools. Fallible resource selection and future DSH permission preparation remain
before input consumption. Current `interaction` projection, authoritative retry
state, public history pagination and transport push routing remain explicit next
steps, not completed behavior. Local Runtime tests still target unpublished
`cd59b6ecbad5ccfb67552660290992662253219b` with a command-line patch.

### Final checkpoint gate (2026-11-09 UTC)

Environment: `RUSTUP_HOME=/workspace/shared/rustup`,
`CARGO_HOME=/workspace/shared/cargo`,
`CARGO_TARGET_DIR=/workspace/shared/minicore-host-observation-target`,
`TMPDIR=/workspace/shared/minicore-host-tmp`. Every Cargo invocation used
`--config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-unbounded"'`
and temporary, uncommitted dependency version/lock selection for 0.10.0.

- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed (with the explicit patch above).
- `cargo test --workspace --all-targets --locked --offline`: passed.
- `cargo test --workspace --all-targets --locked --offline -- --list`: **324 unique test cases**, counted only `: test` lines, not nested storage subprocess output.
- `cargo test --workspace --doc --locked --offline`: **3 doctests passed**.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline`: passed.

The initial lint run caught and corrected unnecessary `Usage::clone()` and the
large notification enum. The concurrent Fault API integration briefly triggered
`result_large_err`; upstream sibling fix `ab4a56b` boxes optional details without
changing its public/wire contract and is included. These checks are local
integration, not remotely reproducible Runtime-0.10 publication or hosted CI.
