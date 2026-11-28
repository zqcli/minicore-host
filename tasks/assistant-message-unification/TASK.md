# Assistant message unification

## Scope and checkpoint

- Baseline: `fdc07b97618504b81a4fb536caf36dcbd4bf456a` (Host 0.13.0).
- Implementation: `0302700756e4987c6c1f4c0d0cbb5418eb76eec1`.
- Worker requested through the subagent interface: `gpt-6-astra`, `max`.
- Version and SPEC unchanged. Local commits only; publication is coordinated by the parent.

## Changes

`ModelMessage::Assistant { parts, provider_replay: Option<ProviderReplay> }`
replaces the separate plain/replay Rust variants. Construction, Debug and
validation now handle one representation; request validation and both provider
adapters consume it directly.

Canonical parts and provider-owned replay remain distinct. Empty parts still
require replay. Chat still rejects foreign replay; Responses still preserves
native item identities and uses its existing projection. Existing serialized
assistant array/object forms are retained at the serde boundary, rather than
changing history/request values as a side effect of this Rust refactor. No new
compatibility route or replay inference was added.

All Host enum consumers were migrated. TUI `c71cb3e186eff5b05612ba85ca940947583541c0`
only matches Runtime User/Tool variants with a wildcard in `src/testing.rs`, so
it needs no matching source migration for this change. Third-party Rust consumers
that match the removed variant must use the unified variant.

## Validation

Rust/Cargo 1.90.0, debug information and incremental compilation disabled.
Commands were run on implementation checkpoint `0302700`:

- `cargo fmt --all --check`: passed.
- `git diff --check`: passed.
- `cargo test -j 3 -p minicore-host-runtime -p minicore-host-provider --all-targets --locked`: passed, 259 tests (147 Runtime, 112 Provider).
- The additional compaction/Core prompt-history command chain was stopped at the integrator's request to avoid contention. Compaction compiled and its 4 unit tests passed; the engine suite was interrupted and Core had not started. Neither suite is reported as fully passed; both belong to the integrated workspace run.

Tests cover replay serde round trips, exact native wire forms, empty-assistant
validation, safe Debug output, tool exchanges, and real local HTTP/Responses/
WebSocket fixture requests. They are not external-model or live TUI acceptance.
Integrated workspace lint/tests and client builds remain the parent task's gate.

## Size and remaining work

Production: +54/-56 lines (net -2). Existing tests: +46/-9 (net +37), primarily
new exact-wire assertions and updated pattern formatting. No benchmark or
performance increase is claimed: this change removes one redundant logical
representation, not the provider behavior that replay requires.

Link this record from the integrated PROJECT checkpoint and record its full
workspace/client validation there. Independent review remains separate.
