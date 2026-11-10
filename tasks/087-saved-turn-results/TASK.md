# Task 087: Read saved Turn results without restoring execution

## Scope and baseline

- Fixed base: `d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06` (tree `34266ef4a0566e3695fe3e3eaa0aad92ef8fb7d2`).
- Isolated branch/worktree: `task/087-saved-turn-results`.
- SPEC v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`, unchanged.
- Runtime: fixed `69faf9196991524b5549499d6208a1eaf567bf56` / 0.10.0, lock/pins unchanged.
- Worker requested configuration: `gpt-6-astra`, `max`; this records the request, not verified effective runtime settings.
- TUI unchanged. No remote actions, push, main integration, or separate result database.

SPEC requires `turn.result` to read the result actually held **or saved** by its Session. Existing execution caches correctly remain empty after restart, but saved `TurnFinished` records were not projected into this query.

## Design constraints

Use existing original TurnId, Input/Runtime/Partial records and public response indexes, plus the saved terminal outcome/reason. The log does not retain LoadId, original config/revisions/counters, or full Fault objects. Do not manufacture them, restore executions, populate completed/output caches, infer outcomes from unfinished records, or associate unbound Summary/Compaction records with a Turn. Existing retained current-run data takes precedence over older saved data. Content reading is asynchronous, cancellation/revocation aware, and outside owner/admission locks.

## Verification

Pending implementation and exact final results. The initial regression is the independent task086 real Host/Runtime, production provider, loopback HTTP/SSE, and actual JSONL probe, copied without weakening its assertion. It is a scripted local HTTP fixture, not a live provider or TUI test.

### Old-source red

On the exact fixed base, the real restart regression failed as expected: the original Turn was completed/persisted with 3 valid JSONL records, cold history was readable, and both cold and explicitly loaded `turn.result` returned `SESSION_TURN_NOT_FOUND` (3201). No active Turn or file mutation was observed. Command: `cargo test -p minicore-host-core --locked --offline --test task087_saved_result_restart -- --nocapture`. Exit 101; 0 passed / 1 failed. Evidence: `/workspace/shared/task087-evidence/old-source-red.log` (worker-local).

All task087 Cargo commands use Rust 1.90, `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, exclusive `CARGO_TARGET_DIR=/workspace/shared/minicore-host-task087-target`, `CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, and `TMPDIR=/workspace/shared/minicore-host-tmp`.
