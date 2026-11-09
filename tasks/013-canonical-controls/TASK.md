# Canonical direct-owner controls

## Scope and baseline

- Baseline: `413916cac0730d291e5c8163cdc3d59e5dbedcd1`, unchanged SPEC.md v4.93.
- Runtime stays pinned to published `18a6ac9b82d003330cd870275efca793cdcc9a96`.
- Implement canonical requests/receipts for existing Agent stop, Session pause/resume/load/unload/close/cancel and exact Turn cancellation. Retain formation-time facts with their actual owners for Host receipt lookup.
- Preserve current authority checks, exact execution identity, save continuation ownership, abandoned-wait behavior, and normal memory reclamation. Do not add Operation entities, deduplication, replay, retries, caches of released bodies, or unsupported method handlers.
- Root manifests, project documents, provider/storage and TUI are outside this worktree's edit scope. This increment does not implement all 52 methods, network authentication, WSS, or TUI.

## Checkpoints

1. Protocol facts (`881624b`, `feat(protocol): define canonical receipt lookup variants`): `SessionCancelReceipt` describes an actually observed current Turn or an idle cancellation, with validated optional identity/state pairing. `Receipt` serializes the original concrete kind and operation ID, without a generic envelope.
   - `cargo test -p minicore-host-protocol --test control_catalog --locked`: passed 15 tests, including idle/active cancellation pairing and unknown-kind rejection.
2. Direct-owner implementation: canonical Host entry methods delegate to Agent/Session controls. Formation and owner retention occur within the same acceptance/state lock as the mutation; shutdown uses the same boundary. Each repeated correlation ID retains distinct invocation facts and singular lookup refuses ambiguity. Lookup excludes internal Session-creation steps belonging to one Agent creation. Session receipts are discarded on actual reclamation, and an immediately reclaimed close still returns its accepted receipt.
   - Snapshots and controls share typed PauseReason/BlockReason facts. BlockReason retains the complete original Fault, with an explicit release condition; metadata/history save fault fields keep their separate meanings.
   - Load/resume return actual loading and pause facts. The original asynchronous loader and save continuation remain owned after caller revocation; a later explicit pause wins. Legacy controls share these primitives. Query DTOs additionally derive Deserialize for the separately implemented typed stdio client.
   - `cargo clippy -p minicore-host-core -p minicore-host-protocol --all-targets --locked -- -D warnings`: passed.
   - `cargo test -p minicore-host-core -p minicore-host-protocol --locked`: passed 156 tests with the eight new owner regressions present in the working tree; final regression commit and workspace gates follow.

## Validation environment

All Cargo commands use Rust 1.90 and:

```
RUSTUP_HOME=/workspace/shared/rustup
CARGO_HOME=/workspace/shared/cargo
PATH=/workspace/shared/cargo/bin:$PATH
TMPDIR=/workspace/shared/minicore-host-tmp
CARGO_TARGET_DIR=/workspace/shared/minicore-host-controls-target
```

Intermediate baseline checks after refactoring:

- `cargo check -p minicore-host-core -p minicore-host-protocol --all-targets --locked`: passed.
- `cargo test -p minicore-host-core -p minicore-host-protocol --locked`: passed the pre-existing 147 unit/integration tests (100 core, 47 protocol); zero doc tests in these two crates.
- Initial compile iterations exposed private Fault field access, changed internal test signatures, and one borrow overlap; fixed before the passing checkpoint above.
- `cargo test -p minicore-host-core --test runtime_owners canonical --locked`: passed eight new owner/Runtime fixture regressions. Full final gates pending.

Tests use actual Host/Agent/Session owners and pinned Runtime with explicit model fixtures and real storage files where applicable, not a live model provider or TUI.

## Next steps

Finish async load/save and creation lookup regressions, run full formatting/lint/tests/docs, record commit hashes, and hand off local commits for independent review. No remote push or main merge authorized or performed.
