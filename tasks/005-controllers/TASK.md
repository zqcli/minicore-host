# Implement Direct Owners

Status: active foundation checkpoint, not full first-version completion.

## Implemented

- Trusted connection handle and non-deserializable caller context. Registry acceptance and invalidation share one short ordering boundary. Read-only connections do not auto-upgrade.
- Host -> Agent -> SessionController routing without a Host Session index.
- Memory Agent/default Session creation, real creation receipts and partial input failure facts.
- Independent next-step/next-turn queues, exact message binding, one active Turn, and same-Turn continuation through successive real AgentLoop owners.
- Configuration snapshots remain fixed for the whole Turn. Pure resource-reference validation precedes commits.
- Explicit pause, cancellation queue ownership, close/unload, Agent stop/start and memory history retention.
- Actual Runtime callback record ownership, separate save state, and retained report on a blocked save.
- Connection-scoped state subscriptions; revoked handles cannot continue reading new state.
- Resource preparation and save callbacks convert panics to owned faults; synchronous input never panics after acceptance because an execution handle is required first.

## Validation at this checkpoint

`cargo test -p minicore-host-core --test runtime_owners --locked`: 15 passed.
`cargo clippy -p minicore-host-core --all-targets --locked -- -D warnings`: passed.

These are real Host/Agent/SessionController/AgentLoop integration tests with explicitly scripted model fixtures, not live provider or TUI tests. Seven regressions were independently reviewed and then incorporated: preparation-close settlement, revocation, stop settlement, invalid config resources, failure pause, preparation panic, and synchronous execution context.

## Local dependency constraint

The core requires the separately developed Runtime history/partial patch. Runtime publication has not been authorized. Local verification uses an ignored `.cargo/config.toml` patch pointing to `/workspace/shared/minicore-runtime-handoff`; the current root lock reflects that local patch. A clean remote checkout is not claimed reproducible until the final Runtime commit is reviewed/published and the Host dependency/lock is finalized. No bundle/bootstrap or copied Runtime source is part of this repository.

## Not implemented / not claimed

The public Host entry currently accepts memory entities only. Durable storage and JournalHistory exist, but persistent Host/Agent/Session startup and lifecycle composition are unfinished. Core controls still need complete typed receipt mapping for the canonical RPC service. Full 52-method coverage, stdio/WSS, DSH, tools/workspace/compaction and real TUI integration are unfinished. There is no implicit success handler for missing features.

Further work: real-file Session save-boundary tests, final Runtime combined-failure integration, full workspace gate, and complete owner lifecycle/transport stages.
