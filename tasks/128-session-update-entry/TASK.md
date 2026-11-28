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
- `cargo test -p minicore-host-core --all-targets --locked --offline --no-fail-fast` on implementation checkpoint: all 20 integration test targets passed, including all 46 persistent-owner tests. The library target had 136 passing and 8 fixture failures: canonical controls reject empty patches that the removed wrapper accepted. `290632e44fe0c839eca2dad5e5fb15e9cdeae0ee` updates those fixtures to explicit fields without changing production validation.
- `cargo test -p minicore-host-resources --test local_tools --test task023 -p minicore-host-tools --test task070_inspect_recovery -p minicore-host-transport --test workspace_sources --test workspace_tool_changes --test workspace_wait --locked --offline --no-fail-fast`: 43 passed.
- `3b37dc57310db7281f2b091942fab733c1f392ea` makes two restart fixtures observe actual owner retirement before reacquiring the root, rather than interpreting a public metadata snapshot as a task-join barrier.
- Final corrected library rerun, workspace lint and integrated Host/TUI checks: parent integration stage. A duplicate worker run was canceled while waiting for the shared build lock, not reported as passed.
- Tests use real owners/files and scripted model fixtures, not live models or TUI.

## Handoff
- Implementation: `058a7d1ef325e53462cc23427cf2c2b63694a732`.
- Test corrections: `290632e44fe0c839eca2dad5e5fb15e9cdeae0ee`, then `3b37dc57310db7281f2b091942fab733c1f392ea`.
- Parent integrates all three and verifies the combined revision before publication. No worker push was performed.
