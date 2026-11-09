# Task 015: Typed and stdio adapters

## Scope and baseline

- Worktree: `/workspace/shared/minicore-host-transport`, branch `feat/typed-stdio-adapters`.
- Baseline: `f38fbebc6c66363e56301f0ad795a8ebc69884b4`.
- Runtime remains published revision `18a6ac9b82d003330cd870275efca793cdcc9a96`; no patch or unpublished Runtime work.
- Ownership: new `crates/host-transport/**` and this task only. Root workspace manifest/lock edits are isolated dependency commits. Core/protocol canonical work is consumed from its own commits, not reimplemented in the adapter.
- Local work only. No remote push, tag, main merge, production binary, provider configuration, WSS or TUI edit.

## Design and implementation

The single typed operation table calls real Host methods for both entry paths.
`HostService` implements `MessageService<CallerContext>`, and `HostClient`
implements `MessageClient`; the in-process path has no JSON roundtrip. Session
contexts contain an address and shared connection only. Creation uses the
existing asynchronous owner-backed creation methods, not the legacy synchronous
memory-only helper. Canonical controls return their direct-owner receipts
without post-operation queries or fact synthesis.

The stdio library serves LF-delimited JSON-RPC over trusted async IO, validating
`initialize {}` before registration and returning the actual `ConnectionInfo`.
It uses original `ConnectionHandle`/`CallerContext` validity, concurrent request
dispatch and one output writer. Connection cleanup cannot promote observers,
rebind old requests, revoke replacement authority or stop accepted owners.
Malformed initialization acquires no right. Notifications never dispatch work.

One client-side monotonic budget spans request preparation through complete
response decoding. Default 30,000 ms and positive overrides share the existing
`CallOptions`/`CallError`; the options are never wire/business parameters.
Timeout removes a waiting response correlation only. No resend, business
cancellation, connection close, operation-retention service, application frame
quota or permanent Host self-cycle was added.

The adapter explicitly routes 25 existing primitives. Unsupported catalog
methods return Fault 1401; absent/ambiguous retained receipts preserve 1202/1203.
Partial creates and accepted/start-error facts remain full normal RPC results.
See the crate README for exact methods, parameter layout and limits.

## Commits

- `80899d1`: isolated crate/workspace bootstrap and locked dependencies.
- Canonical dependencies: source `881624b`, `f10d859`, `8a68edb`, `fcf3580` and receipt-fault correction `b961f9f45ea31def2e215fa07368c05893cdf6fa` (locally cherry-picked as `d174593`, `0d52def`, `74fdb6d`, `9c77d8a`, `f5dec46`). Do not duplicate these when integrating into the parent branch.
- `d7b6cd5`: isolated removal of unused process/temporary-file test dependency features; real OS pipe tests need only Tokio net/test-util. No production fixture executable was added.

## Verification so far

Environment for every Cargo command:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-transport-target
```

- `cargo check -p minicore-host-transport --offline`: passed.
- `cargo test -p minicore-host-transport --offline --no-fail-fast`: passed 15 tests (12 real-owner adapter/OS-pipe cases and 3 explicitly delayed transport-peer clock/error cases), no ignored tests.
- `cargo fmt -p minicore-host-transport`: applied only to the owned crate.
- `cargo clippy -p minicore-host-transport --all-targets --locked --offline -- -D warnings`: passed.
- Full final workspace fmt/clippy/test/doc gate and repeat runs: pending the final checkpoint below.

The first large-frame test exceeded the existing Runtime 256 KiB text structural
limit and correctly received `SESSION_INPUT_INVALID`. Its fixture was corrected
to 256,000 text bytes (384,000 escaped JSON body bytes), proving the transport
adds no smaller framing quota without changing the Runtime business contract.
An initial stop test incorrectly expected completed shutdown; it was corrected
to accept the actual `stopping`/`stopped` receipt and owner state.

## Tested contracts and limits

Coverage includes typed/RPC query DTO parity, MessageService and actual
Host→Agent→SessionController→Runtime execution, OS-pipe framing/disconnect,
partial responses and single default/override deadlines, no resend, concurrent
long wait plus cancel/query, slow creation plus unrelated traffic, accepted
creation after timeout/abandonment/disconnect, obsolete caller acceptance,
read-only reconnection, JSON authority rejection, malformed RPC/initialization,
notifications, partial creation/start-error results, all 25 routed methods,
receipt lookup distinctions, explicit close and last-client-drop cleanup, and
shutdown receipt before runtime completion. Model responses and intentionally
delayed transport peers are labeled fixtures, not live integration evidence.

Not implemented/not run: WSS and key rotation, full §9 event subscriptions and
output routing, remaining §8 handlers, full process configuration, a production
Host executable or child-process stdio boot, live-provider calls, TUI integration,
network deployment and cross-process provider credentials. Existing query
primitives still lack complete public pagination/history envelopes. This is not
a claim that all 52 methods or the full first-version specification are done.
