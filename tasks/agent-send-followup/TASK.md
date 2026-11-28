# Agent send scheduling

- Baseline: `fdc07b97618504b81a4fb536caf36dcbd4bf456a`, Host 0.13.0.
- Implementation: `9a5db4038be85a0944955d4f62364de4328c2260`.
- Requested worker: `gpt-6-astra`, `max`; no independent effective-runtime
  attestation is available. Shared constraints are in `AGENTS.md`.
- User-approved change: restore explicit `send.mode: follow_up`; omission
  remains `steer`. No version bump, create-path change or new scheduling layer.

## Implementation and behavior

`host-tools/src/tool.rs` now forwards the parsed mode instead of replacing it.
Existing scope and owner delivery paths already preserve the input mode.
`schema.rs`, the crate README and SPEC's action/parameter tables now agree.
Explicit Session and Agent-default sends preserve the mode. Explicit-Session
delegation and appending to an existing relation do too.

Use `steer` for active coordination and answers to a running peer's `wait`.
`follow_up` intentionally waits for a new Turn; it cannot unblock that wait.
Completion notifications retain their existing routing and lifetime rules.

## Validation

Linux x86_64, Rust 1.90.0, implementation commit above:

- `cargo fmt --package minicore-host-tools -- --check`: passed.
- `cargo test --locked -p minicore-host-tools --test adapter --test contracts --test delegation_acceptance_fences`:
  37 tests passed.
- `cargo test --locked --offline -p minicore-host-tools --all-targets`:
  80 tests passed.
- `git diff --check`: passed.
- `cargo clippy --locked --offline -p minicore-host-tools --all-targets -- -D warnings`:
  passed.

The adapter fixture covers omitted/explicit modes for plain sends, enrollment
and relation append. Real Host/Runtime with a scripted model verifies default
steer waking an active wait, explicit follow-up staying out of that Turn,
consumption in the next Turn, and first delegated follow-up to a busy Session.
Existing create, cancellation, scope, persistence and notification tests pass.
No live provider, TUI, macOS or Windows validation is claimed here.

## Handoff

No push performed. Parent integration owns the final SPEC version/hash,
PROJECT checkpoint, cross-task full-workspace validation and publication.
