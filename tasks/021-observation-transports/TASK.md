# Observation transports and owner answers

## Scope and source

Implement SPEC v4.93 §8–11 through the existing owners: typed clients, real
stdio/WSS push, scoped subscription lifetime, authoritative state/range/history
reads, and narrowly scoped control/interaction integration. TUI is unchanged.
No remote push is performed by this task.

The worktree starts at published-runtime Host `2751106` and cherry-picks the
observation access bridges `1a87f53`, `0eeb57b`, then observation sources
`da7c51e90324797c9b1be759a8bbb53469392df5`,
`49af8f89d67d741d445329e6a66f620086d98ef9`, and
`39a138ec42ea1abddd3c33427e79032f1e3e0426`. Those prerequisites are not this task's
new source commits and must not be replayed twice during parent integration.

## Stage 1 implemented

- Thirty actual typed/stdio/WSS routes, including events subscribe/unsubscribe,
  model/tool output reads, real ToolCall reads and paginated history. `turn.get`
  uses the same atomic baseline factory as state events and initial snapshots.
- One writer per RPC connection; real core subscriptions own their scopes.
  Notifications have separate JSON-RPC decoding. Early pushes and early resync
  are correlated by the real subscription ID, including intersecting setups.
- A dropped/timed-out setup is cleaned up when its late result identifies the
  actual subscription. Cancellation after reader registration is also covered.
  Dropping/ending one observation never closes a shared connection or cancels
  accepted work. Established streams have no ordinary-call timeout.
- Optional `OutputBuffer` retains only client-received bytes for one exact
  source/channel. Fixed gaps, UTF-8 page boundaries, overlapping late ranges,
  baseline overlap and completion-before-tail remain distinct from source
  execution and persistence. Missing owners return Fault, not empty success.
- Real Host/Runtime integration through OS pipes and loopback TLS covers dynamic
  descendants, multiple/read-only scopes, source reclamation, a stopped pipe
  reader, genuine early frames, file-tool input/output/results, and identical
  persistent history with/without observers. Cold history creates no Runner or
  old Turn. Unit-level resync correlation is explicitly separate from those
  transport tests. No live provider or TUI integration is claimed.

## Stage 1 validation (2026-11-09 UTC)

Environment:

```
RUSTUP_HOME=/workspace/shared/rustup
CARGO_HOME=/workspace/shared/cargo
PATH=/workspace/shared/cargo/bin:$PATH
CARGO_TARGET_DIR=/workspace/shared/minicore-host-event-transport-target
TMPDIR=/workspace/shared/minicore-host-tmp
CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_TEST_DEBUG=0
CARGO_INCREMENTAL=0
```

Every Cargo build/test command below additionally used this explicit command-line
patch, with Runtime HEAD `3cf57e3e1947b0464bd8843ed0508bfddfb2639a`:

```
--config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-retry-state"'
```

The temporary patch lock selection is not committed. Runtime manifests still
pin the verified remote 0.10.0 publication `59ee8ce668f72a21d408e6247d3ce075b260db14`;
the observation prerequisite requires the pending retry-state publication for a
fully remote-reproducible final build. Do not encode a local SHA as a remote rev.
The only committed lock additions are observation's `time`/serde feature closure
and the already pinned `tempfile = 3.27.0` transport test dependency.

- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- `cargo test --workspace --all-targets --locked --offline`: passed.
- `cargo test --workspace --all-targets --locked --offline -- --list`: **368 unique tests**. Count `: test` list entries; nested storage subprocess output is not another test.
- `cargo test --workspace --doc --locked --offline`: **3 doctests passed**.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline`: passed.

Development failures were corrected and rerun: old tests used newly implemented
`events.subscribe` as an unavailable-method fixture; the initial watermark test
incorrectly assumed the first channel was text instead of selecting it; snapshot
comparisons now allow the owner's real advancing Runner revision between reads;
a test helper type alias resolved Clippy's complexity warning. No failure was
reclassified as success.

## Remaining stage 2

- Dynamic Host control-state notifications on actual connection close/new right
  acquisition/key rotation, without promoting surviving read-only connections.
- Real `interaction.answer` forwarding to the exact Runtime waiting point,
  operation receipt, strict identity/answer validation and cancellation ordering.
- Independent review and parent integration, then the parent's verified remote
  Runtime retry pin and full gate on that final source. No network publication,
  live-provider behavior, production CLI or TUI changes are included here.

## Control-change follow-up

Stage 1 source commit: `5b17dc85c41218ff04f41d586263fcae7eea7e28`.
The control notification patch uses the private Weak Host attachment from DSH
source `6536da608f7e97a72a02221695112bf74fb33193` as a prerequisite. Parent
integration should take only this task's control-change commit, not replay DSH.

Connection register/close and key publication now publish the actual current
`host.state` after releasing the registry mutex. Network revocation still closes
only original network scopes. Surviving local/read-only connections are not
promoted; only a new registration can acquire the vacant right. No Host state
lock is held across child calls or transport I/O.

Validation with the same explicit local Runtime patch: formatting passed;
`cargo clippy -p minicore-host-core -p minicore-host-transport --all-targets
--locked --offline -- -D warnings` and the matching all-targets tests passed.
The new real loopback TLS test covers current key invalidation, local observer
survival, occupied/unoccupied revisions, read-only rejection, fresh authority,
ordinary disconnect, and completion of previously accepted Runtime work.

## Independent review correction: rejected unsubscribe

Review found that `HostClient::unsubscribe` ended its local stream even when
`CallOptions { wait_timeout_ms: Some(0) }` was rejected before sending anything.
A real OS-pipe regression failed before the fix: its next `recv` was `None`, while
a subsequent valid unsubscribe returned `ended=true` from the still-live Host.
The SDK now removes a local stream only after a successful unsubscribe result.
Failure/timeout does not claim remote completion and is not retried.

`cargo test -p minicore-host-transport --test unsubscribe_validation --locked
--offline` (same explicit Runtime patch) passed after the two-line fix. The test
is retained permanently. This review correction is separate from the pending
interaction source changes.

## Broad-scope reclamation race

A Session may be reclaimed between Host/Agent descendant enumeration and its
baseline read. Only those broad live scopes now skip the owner's explicit
`AgentSessionNotFound` result for that descendant. Exact Session/Turn subscriptions
still fail, other errors are unchanged, and the final connection-read check is
retained. The focused test holds an actual enumerated Session Arc, closes and
waits for real memory reclamation, then checks both scope behaviors. No source
cache, lifetime extension or fabricated empty exact-source result is introduced.

`cargo test -p minicore-host-core enumerated_descendant_reclaimed_before_baseline
--locked --offline` passed with the explicit Runtime patch. This correction is
independent of interaction answer routing.

## Interaction contract

The dedicated protocol change adds generic `InteractionAnswerRequest<A>` with
original Turn/Load/InteractionId and OperationId, a strictly typed answered
receipt in the common `Receipt` union, and stable `SESSION_INTERACTION_NOT_PENDING`
(3601) / `SESSION_INTERACTION_ANSWER_INVALID` (3602) leaves. The generic payload
lets core use Runtime's existing tool-input answer type without making protocol
depend on Runtime or copying another input schema. An answered receipt has no
execution-completed status. All fields and receipt branches reject unknown shape.

`cargo test -p minicore-host-protocol --all-targets --locked --offline` passed
with the same explicit patch. Two new contract tests cover mandatory execution
identity, unknown/null fields, receipt round-trip and invalid completion claims.
The following source change supplies the actual owner/Runtime route.

## Actual answer routing and final local gate

Protocol source: `7ce7aaf4e98c7d383cd8b4b1b2f344576fb0a6d6`.
The owner route now resolves the public 20-character interaction ID through the
original Session's mapping to the actual Runtime ID. The Runtime's own answer
validation and answer/cancel decision settle that exact slot. No projection can
stand in for an answer. Both Runtime Approval and ToolInput are supported.

The Session/history owner explicitly handed off these narrow shared hunks:
`mod interaction` and reexports, a private per-record answer fence, visibility of
existing receipt retention, and one lock-free fence wait before final
`active.take`. All other drive/history/retirement logic remains with its owner.
The synchronous `LoopHandle::answer` executes outside Session and registry locks.
After real acceptance, answered is published once and the original receipt is
retained. Final cancellation settlement waits for any already accepted answer
attempt without adding public states, a new task layer or observer retention.

Real typed/OS-pipe/TLS tests cover operator replacement, unchanged pause,
read-only rejection, wrong Load/Turn/interaction, malformed/invalid choice,
answer-before-cancel and cancel-before-answer, two concurrent Session waits with
reused ToolCall IDs, and real tool execution/denial following Approval. ToolInput
produces the Runtime's real InputProvided ToolResult. No transcript projection,
live provider or TUI success is substituted for these actual owners.

Final commands used the environment and explicit local Runtime patch documented
above; manifests/committed lock still contain no local path or unpublished rev.
The parent reports the matching Runtime tree published as
`69faf9196991524b5549499d6208a1eaf567bf56` and owns the final remote pin/check.

- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- `cargo test --workspace --all-targets --locked --offline`: passed.
- `cargo test --workspace --all-targets --locked --offline -- --list`: **388 unique tests**.
- `cargo test --workspace --doc --locked --offline`: **3 doctests passed**.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline`: passed.

This count includes local DSH prerequisite `6536da6` and this task's 21 added
cases relative to its observation prerequisites; it is not the parent's larger
integration count. The independent partial-history review finding is delegated
to task 022: public history must include actual saved partial records rather
than Runtime model-context projection alone. That separate fix and independent
review/final parent integration remain required before claiming the complete
observation surface ready. No Runtime, Host or branch was pushed by this worker.

## Review correction: preserve the original interaction deadline

Review of integrated `713efb0` found that a real Runtime deadline settled a
pending interaction as `cancelled` rather than the `expired` fact required by
SPEC section 5. The retained `LoopReport` already distinguishes
`CancelReason::Deadline`; no new timer or deadline service is necessary.
The final observation now reads that original report and publishes `expired`
only for unresolved interactions actually ended by that deadline. The Turn
remains `Cancelled`, explicit cancellation remains `cancelled`, and previously
accepted answers remain `answered`. The only history finalizer change passes
its existing report to the observation projection; no history DTO or save
behavior changes.

Validation now uses the published Runtime
`69faf9196991524b5549499d6208a1eaf567bf56` directly, without a command-line patch.
Independent bootstrap `87ed77a6c81518609e71e54d67c4713bec52ed34` changes only the
four Runtime manifest revisions and matching lock source; the parent already
owns that pin and need not replay the bootstrap.

- `cargo test -p minicore-host-transport --test interaction_answers
  original_deadline_expires_pending --locked --offline`: reproduced red before
  the fix (`cancelled` instead of `expired`).
- `cargo test -p minicore-host-transport --test interaction_answers --locked
  --offline`: **8 passed**, including three new real OS-pipe/Runtime cases.
  Each covers both Approval and ToolInput, one settlement, actual tool outcome,
  pending cleanup and rejection of a late answer. The cases distinguish pending
  expiry, answer before the original deadline, and explicit cancellation before
  that deadline.
- `cargo fmt --all --check`: passed after formatting.

Logs: `/workspace/shared/event-deadline-red.log` and
`/workspace/shared/event-deadline-green.log`. Full aggregate validation follows
the independent pipe teardown correction. These are scripted-model owner tests,
not live-provider or TUI validation. No remote push was performed.

## Review correction: deterministic pipe teardown

The resource stage reproduced `BrokenPipe` at the persistent-history test's
server join. This was a test teardown assumption, not a storage or owner failure:
dropping its observation queued a real `events.unsubscribe` request, immediately
followed by `client.close()` closing both directions. The server could write the
unsubscribe response after its reader was gone. Terminal business state does
not promise a drained transport, and stdio correctly reports that OS error.

The persistent observed/unobserved comparison now closes only the request pipe
first, retains the actual response reader through the server's EOF exit, then
closes the SDK and drops the observation. A test-only byte-copy relay permits
this half-close; every business request and response still crosses real OS
pipes. All saved-history equality, cold reads, unloaded state, missing old Turn
and no-new-Runtime assertions are unchanged. Production shutdown semantics and
I/O error handling are unchanged.

The separate permanent gated regression captures the actual Host unsubscribe
response with `ended=true` before its write. With the request pipe kept open,
closing the response reader first deterministically yields exactly `BrokenPipe`.
Closing the request pipe first while retaining the reader deterministically
returns `Ok`. Both paths then finish a previously accepted real Runtime Turn
and read both its complete body and the previous Turn's history. This does not
swallow arbitrary errors, abort the server, retry until green, or fabricate a
peer response. The gate delays only a genuine server write.

Final source includes deadline correction
`46bd4c9a916eb7c394673b63f2c89ddec9d6fc4d` on the published Runtime pin from
bootstrap `87ed77a6c81518609e71e54d67c4713bec52ed34`. No path patch is used.
The parent's integration already contains the dependency pin and separately
owned history/resource fixes; only the two review-fix commits should be replayed.

Commands use the environment above, without `--config` overrides:

- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- `cargo test --workspace --all-targets --locked --offline`: passed.
- `cargo test --workspace --all-targets --locked --offline -- --list`: **392 unique tests**.
- `cargo test --workspace --doc --locked --offline`: **3 doctests passed**.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline`: passed.
- Focused `observation_tools` (2) and `observation_shutdown` (1): passed.

Aggregate logs: `/workspace/shared/event-review-final-{clippy,workspace,list,doctests,rustdoc}.log`.
Deterministic pipe log: `/workspace/shared/event-pipe-teardown.log`.
The count is this worker's branch, not the parent's later integrated suite.
Independent verification and parent integration remain pending. No remote push
or unrelated shared history DTO/Host resource modification was performed.
