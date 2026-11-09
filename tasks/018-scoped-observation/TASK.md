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

## Authoritative state and real history follow-up

Separate local Runtime candidate: `3cf57e3e1947b0464bd8843ed0508bfddfb2639a`
at `/workspace/shared/minicore-runtime-retry-state`. Its parent `142412b` adds
the authoritative state handoff; the last commit adds tests/docs only. These
changes are not silently included in the previously published Runtime pin.
Bootstrap `49af8f8` pins `time = 0.3.55` with serde/formatting/parsing for strict
RFC3339 values; no unrelated dependency upgrade is requested.

- Runtime state/watch, rather than public output events, supplies current retry
  phase, actual Model::start counts, source time and original response identity.
  Session always drains output and joins the real Runtime, with or without any
  subscribers. Terminal state clears the retry field.
- Source-owned interaction mapping translates actual Runtime identities into
  stable Host-format 20-character IDs. Initial state, request and resolved
  events reuse that mapping. Actual cancellation/failure settles remaining
  interactions after the Turn joins and saves; it never fabricates an answer.
  Public answer routing remains future integration; the original owner retains
  the mapping for that entry point.
- History pagination adapts actual owner-held history or uses
  `SessionStore::read_history` for cold persistent content. It creates no Runner
  and restores no old execution; the real-file restart test confirms unchanged
  unloaded residency, rejected old cursor and missing old Turn/output source.
  In-memory pagination clones only the requested page.
- Session preparation does not emit a registered-child snapshot. Observation
  starts after actual insertion; discovered persistent Sessions are registered
  in the same way.

New deterministic tests cover completion before tail bytes, retry waiting
cancellation, requesting/clear with actual attempt counts, interaction request
and cancel settlement, real persistent cold history, unpublished-child gating,
and Agent/Turn dynamic descendants. No sleeps guess race order. All earlier
observation scenarios remain covered. No public events reconstruct durable
history; final tool results are still separate from actual process output.

Known remaining integration: typed/stdio/WSS subscription push and read routes;
public interaction answer routing; Host state notifications when connection
control changes on close/key rotation (the authoritative query and initial
baseline already reflect current control). No new global event sequence,
capacity quota, permanent replay store, ACK or business fast path was added.

### Follow-up gate (2026-11-09 UTC)

Same environment as above; the command-line patch path is now
`/workspace/shared/minicore-runtime-retry-state`, fixed HEAD
`3cf57e3e1947b0464bd8843ed0508bfddfb2639a`. Temporary Runtime version/lock changes
remain uncommitted. `cargo fmt --all --check`, workspace all-targets Clippy with
`-D warnings`, workspace all-targets tests, all 3 doctests and rustdoc with
`RUSTDOCFLAGS='-D warnings'` passed using `--locked --offline` and that explicit
patch. `cargo test --workspace --all-targets --locked --offline -- --list`
contains **332 unique test cases** (13 scoped-observation cases, 4 observation
protocol cases). The final extra assertion that public interaction IDs use the
20-character Host format also passed its focused real Runtime case. No test
claims live external model, TUI, remote Host CI or published retry-state source.

During development the retry fixture used an invalid 60-second configured base
(backoff maximum is the existing 30 seconds), so the model never started. The
fixture was corrected to the valid 30-second base and cancellation is gated by
the observed real waiting state, not elapsed time. Protocol round-trip testing
also caught serde flatten/deny-unknown-fields interaction; retry wire branches
now explicitly validate their exact fields and RFC3339 timestamp. Neither test
failure was concealed as a successful production path.
