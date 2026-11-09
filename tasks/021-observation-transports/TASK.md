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
