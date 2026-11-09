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

## Milestone 2: real transport and Runtime integration

Milestone 1 code commit: `67b89ad5c53260df177df267b076724658b82ac8`.

Implemented loopback TCP HTTP/SSE fixtures that call the real `OpenAiChatModel` and actual `AgentLoop`. Coverage includes request/system/user/tool-history encoding; fragmented ID/name/arguments; parallel tools and one-time execution; text/reasoning and Unicode event splitting; disjoint usage plus exact provider counts; missing/invalid usage; all actual finish mappings; truncated/malformed stream failure; cancellation and actual socket close; no SDK retries; recognized pre-start rejection versus post-role/output errors; Retry-After; redirects; bad content type; unsupported reasoning history; and pre-cancelled no-send.

Added partial preservation using Runtime's real report: incomplete tool arguments remain non-executable; ID/name fragments before a valid ToolCallStart are kept only in existing ModelPartial.provider_replay; reported usage and actual finish metadata survive later truncation. Unknown tool finish reasons never authorize tool execution.

Integration discovered that original Runtime discarded a complete Unknown/no-tools response. The Runtime owner fixed this at `5e61c22da4b88891249902ed88184b1eca334504`, preserving complete AssistantHistory and callback before Failed(InvalidModelResponse). The provider's new end-to-end regression passes against that exact real Runtime revision. This is not a fake Runtime or adapter rewrite to Stop.

Validation, using the same manifest and command-line Runtime path patch shown above:

- `cargo fmt --check`: PASS.
- `cargo test --locked`: PASS, 5 unit tests, 22 HTTP/SSE integration tests, 1 compiling documentation example.
- `cargo clippy --locked --all-targets -- -D warnings`: PASS.
- `RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps`: PASS.
- `git diff --check`: PASS.

## Integration handoff and limits

The public constructor takes exact endpoint, provider model, actual ModelDescriptor and a nonserializable secret (or its environment-variable name). No application output cap is defaulted. Bind the resulting `Arc<dyn Model>` in the core resource resolver; do not copy credentials into public model/session values.

The isolated crate manifest pins the original Runtime API baseline; its local lock reflects the command-line handoff patch. The root integrator must carry Runtime `5e61c22da4b88891249902ed88184b1eca334504` and own the final workspace dependency/lock strategy. Integration tests using ModelPartial are not claimed to compile against the unmodified earlier Runtime.

Remaining beyond this assigned adapter scope: full Host→Agent→Session→provider integration (owned by parent), independent review, authorized live provider testing, Responses/all-provider parity, multimedia and opaque provider reasoning support, and unchanged TUI integration. No remote push was made.
