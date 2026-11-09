# Implement production model transport

## Ownership and baseline

- Owned paths: `crates/host-provider/**` and this task log only.
- SPEC v4.93 §§2, 5, 11; direct Host → Agent → SessionController → Runtime ownership unchanged.
- Runtime API baseline: `3fd5ecda1fb7c717ab0a39218d90268b4d97909c`.
- Selective adapter reference: Agent `f36a87366d8e2fe0a1c8216463887902e471c739`; provenance and dual-license declaration in crate `PROVENANCE.md`.
- No external model endpoint or real key was used. No remote pushes authorized or attempted.

## Milestone 1: transport implementation

Implemented exact-endpoint HTTPS/Bearer Chat Completions Model, nonserializable secret/config, real HTTP/SSE, explicit reqwest retry/redirect disablement, cancellation/deadline, request and tool-history encoding, text/reasoning/tool deltas, exact provider finish metadata, conservative provider error classification, and usage normalization. No default output cap/application quota or synthetic successful completion. Unsupported provider formats fail explicitly.

Validation environment: Rust 1.90; `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, Cargo on PATH.

Commands (from repository root):

- `cargo fmt --manifest-path crates/host-provider/Cargo.toml`
- `cargo check --locked --manifest-path crates/host-provider/Cargo.toml --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-handoff"'`: PASS.

The isolated crate lock was seeded from the pinned Agent lock because crates.io rejects newly resolving yanked tokio 1.54.0. It retains upstream-locked exact dependency versions. The root integrator owns the final workspace Cargo.lock and Runtime revision.

- `cargo test --locked --lib` with the same manifest and path patch: PASS, 4 tests.

## Pending

Real loopback HTTP/SSE integration through AgentLoop, adversarial stream/retry/cancel tests, lint/docs, and final review. Live provider, all-provider parity, and TUI integration are not claimed by this adapter milestone.
