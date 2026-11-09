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

## Accepted creation checkpoint

- Added asynchronous creation entries for both modes over the existing owners and Runtime path. Host and Agent retain typed creation progress and the actual task handles; a caller abort or connection invalidation only ends its wait.
- Persistent Agent publication follows its own confirmed save. Agent snapshots Session defaults at acceptance, saves Session files, confirms a reloadable default reference, then publishes the Session. Incomplete pre-publication files are retained on failure, not reported as published references or implicitly rolled back.
- P/P, P/M and M/M use the proper backends; M/P is rejected. Agent stop includes accepted in-flight creation, and late publication stays disabled rather than starting new work.
- Tests cover actual file failure after Agent publication, real partial initial-message rejection, restart without memory objects/old Turn state, and waiter abort plus connection revocation while accepted work continues through real Runtime/persisted output.

Commands: `cargo fmt --all`; `cargo test -p minicore-host-core --offline` (28 passed: 4 unit, 8 persistent owners, 15 memory owners, 1 real HTTP integration); `cargo clippy -p minicore-host-core --all-targets --offline -- -D warnings` (passed).


## Root lease review correction

Independent review reproduced an early directory lease release: memory descendants of a persistent Host retained execution but not the root guard. All descendants now retain a type-erased storage lease, without holding Arc<Host> or acquiring cross-owner business authority. A gated real Runtime regression verifies that dropping Host/connection cannot reopen its directory while the accepted memory execution remains active.

Verified the isolated root-lease commit tree: `cargo fmt --all`, full core tests (29 passed) and core all-target Clippy with warnings denied all passed. This verification excludes still-uncommitted metadata/lifecycle work.
