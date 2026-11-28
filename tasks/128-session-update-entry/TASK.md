# Single public Session update entry

## Scope
- User-approved public Rust API consolidation on `fdc07b97618504b81a4fb536caf36dcbd4bf456a`.
- Keep `Host::session_update` and unchanged WSS `session.update` request/receipt shapes. Remove `Host::update_session` and `Host::update_session_async`.
- Remove the now-unused internal patch wrappers. Keep `SessionController::update_candidate_async` used by Agent tool updates, which waits for actual storage completion.
- No version bump, push, storage API removal or Agent update API change in this worker.

## Implementation and semantics
- Main changes: `host-core/src/host.rs`, `host/metadata.rs`, `session/metadata.rs`.
- Public callers use operation IDs, expected revision and `SessionUpdateChanges`. Persistent controls still return `Saving` before storage completion; receipts remain immutable.
- Tests migrate to the public entry. Repeated persistence checks share a test-only observer helper; scheduling gates now queue disk work after canonical validation. Memory/no-executor regression exercises the retained candidate path; authority, stale revision, lifecycle, current-Turn binding and metadata failure coverage remain.

## Validation checkpoint
- Requested worker model: `gpt-6-astra`, reasoning `max`; runtime effective settings are not independently introspectable.
- `cargo fmt --all`: passed.
- `git diff --check`: passed.
- `cargo test -p minicore-host-core --all-targets --locked --offline --no-fail-fast`: running at this checkpoint. Final outcomes will be recorded in a follow-up commit.
- Other workspace/integrated tests: parent integration stage.
- Tests use real owners/files and scripted model fixtures, not live models or TUI.

## Handoff
- This file's containing commit is the implementation checkpoint. Parent is integrating; do not publish a success claim until checks finish.
