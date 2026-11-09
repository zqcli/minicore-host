# Compose Persistent Owners

Status: active, not full first-version completion.

## Scope

Compose existing real object storage and journal with Host -> Agent -> SessionController, preserving the same Runtime path and queues as memory owners. Support stable Host identity, fresh incarnation, hierarchical discovery, lazy history loading, real creation receipts, accepted-work ownership, persisted configuration and close/default ordering.

HostSettings is only currently supported identity metadata (name). ResourceResolver remains explicit application assembly. Full Host resource/network/log configuration, reload/shutdown, canonical service coverage, DSH, transport, interaction and compaction are outside this task.

## Verification and provenance

Commands and commit checkpoints are recorded below when run. Local validation uses an ignored Cargo patch to the separately unpublished Runtime extension. No Runtime copy, bundle, local path or root lock changes are committed. Clean remote reproducibility is not claimed.

## Stable identity and discovery checkpoint

- Real directory ownership precedes Host/Agent/Session construction. Host settings and stable ID reload unchanged; each run receives a new incarnation.
- Agent initialization reads its own metadata, then loads each Session's metadata independently. Session history is lazy; a bad default Session stays unavailable and does not redirect or hide healthy siblings.
- Real temporary-file Runtime tests cover exclusive roots, unchanged settings, lazy startup, history save boundaries, corrupted history and damaged default metadata isolation, and rejection of unknown stored fields.
- Persistent mutating convenience methods explicitly reject until the asynchronous owner path is installed; no persistent operation silently uses MemoryHistory.

Commands: `cargo fmt --all`, `cargo test -p minicore-host-core --test persistent_owners --offline` (5 passed), `cargo clippy -p minicore-host-core --all-targets --offline -- -D warnings` (passed). Full core tests at this checkpoint: 25 passed plus empty doc-test suite.
