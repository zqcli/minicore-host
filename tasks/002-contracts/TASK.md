# Implement Public Contracts

Status: active. Scope: `crates/host-protocol/**` and this task file only.

Baseline: SPEC v4.93, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` verified unchanged.

## Milestone 1: identity, ownership, configuration, faults

- Strong random 20-symbol IDs use the exact alphabet and OS CSPRNG. Provider ToolCall IDs remain unchanged and Turn-scoped.
- Flat direct-owner references, storage-mode matrix, lifecycle/outcome/persistence values.
- Strict complete configuration, explicit create/update patch semantics, nullable workspace clearing, immutable candidate construction and owner-provided resource validation. No application quota fields.
- Existing Runtime reference grammar and retry/deadline ranges retained explicitly; Role is not a permission or execution identity.
- Single fault registry preserves all SPEC codes and adds named owner-boundary faults; strict type/code/details/retry validation and distinct client-side errors.
- Exact dependency pins match the Runtime reference lock: getrandom 0.4.3, serde 1.0.229, serde_json 1.0.154. Temporary standalone crate workspace/lock; parent integrates the final root Cargo.lock.

Validation (Rust 1.90.0, workspace-local official toolchain):

```sh
export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo PATH=/workspace/shared/cargo/bin:$PATH
cargo fmt --manifest-path crates/host-protocol/Cargo.toml
cargo test --manifest-path crates/host-protocol/Cargo.toml
cargo clippy --manifest-path crates/host-protocol/Cargo.toml --all-targets -- -D warnings
```

Tests: 1 internal uniform-encoding test and 13 integration tests passed. Clippy initially found a derivable Default implementation; fixed before the milestone commit and rerun. No remote writes or provider/TUI claims.

## Next

Strict text requests and typed receipts, connection/control facts, exact 52-method catalog with explicit staged availability, additional positive and negative wire tests.
