# Implement Direct Owners

Status: memory-controller foundation and real Runtime integration complete; persistent controls continue in task 006. The full first version is not complete.

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

## Runtime dependency provenance

Initial controller verification used an ignored Cargo patch to the then-unpublished Runtime history/partial worktree. After the owner approved publication through the GitHub interface, commit `f1ec37e` pinned the verified remote Runtime `18a6ac9b82d003330cd870275efca793cdcc9a96` in all manifests and the root lock. The patch and unused vendor symlink are removed. The complete Host gate also passed in a fresh local clone with an empty target directory at `d73b161`; see task 001 for exact commands and cache boundaries. No bundle/bootstrap or copied Runtime source is part of this repository.

## Not implemented / not claimed

The initial checkpoint accepted memory entities only. Task 006 subsequently added persistent startup, accepted creation, confirmed metadata changes, load/resume, durable close/default ordering and associated owner-lifetime corrections. Core controls still need complete typed receipt mapping for the canonical RPC service. Full 52-method coverage, stdio/WSS, DSH, tools/workspace/compaction and real TUI integration are unfinished. There is no implicit success handler for missing features.

Further work: complete the remaining Host-level lifecycle and canonical service/transport stages. Real-file boundaries and combined-failure tests are completed below; aggregate gates and their fixed commits are recorded in task 001.

## Save and production-adapter integration checkpoint

Against Runtime local commit `49a6e8b06512616a2fb9c6104c07536b8396c4ca`:

- `cargo test -p minicore-host-core --lib --locked`: 4 passed. Real temp-directory JSONL confirms response-before-tool, tool-batch-before-next-model, and final-save-before-public-finished barriers. Cancellation retains the save owner and actual complete response. Truncated output persists as a distinct partial. Simultaneous stream/save failure retains the primary fault, separate persistence fault, full authoritative report and blocked finalization, without duplicate append or fake finished.
- `cargo test -p minicore-host-core --test http_host --offline`: 1 passed. Two turns traverse public Host -> Agent -> SessionController -> real Runtime -> production reqwest/SSE -> real loopback TCP fixture. Exact user history and one framework/Role injection per request are checked. This is not a live external model or TUI test.
- `cargo test -p minicore-host-core --test runtime_owners --locked`: 15 passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed after correcting two test-only lint issues (explicit default initializer and lexical mutex-guard scope).

Independent targeted review re-ran the final non-Tokio regression and confirmed it closed. Full external review remains pending; main has not been merged.

## Multi-thread scheduling checkpoint

- Most owner regressions now run on four Tokio worker threads. The cancellation claim/send/repeat test retains a current-thread executor to keep that synchronous sequence inside one declared schedule boundary.
- Added the opposite finish/steer ordering: after an actual first-Turn final claim, a new steer receives a new Turn without a lost wake, duplicate finish or previous-Turn rebinding.
- Replaced fixed yield-count assumptions in close/preparation/failure tests with actual state/Turn settlement waits. Idle Agent stop uses a bounded condition wait rather than assuming ten yields completes an owner task.
- `cargo test -p minicore-host-core --test runtime_owners --locked`: 16 passed.
- Repeated the same 16-test suite 25 times: every run passed. This is repeated scheduling coverage, not 400 distinct cases or a capacity claim.
- `cargo clippy -p minicore-host-core --all-targets --locked -- -D warnings` and formatting passed. No production source changes were needed.

- Additional release-profile check at Host `4b8de50`: `cargo test -p minicore-host-core --release --test runtime_owners --locked` passed all 16 cases. This is not a performance benchmark.

## Default selection and revocation review regressions

The final review's memory default cases are now permanent tests: both set/close orders, 128 concurrent set/close races evaluated against the actual accepted result, accepted defaults surviving revocation, and old/read-only callers remaining unable to mutate. A unit fixture holds only the child's state lock while the real default precheck waits, proving that connection revocation remains available and the eventual old-caller acceptance fails. It covers the precheck move in `d2c41ca`; the Agent's closing-set check still owns the final publication boundary.

The integrated complete gate passed 198 unit/integration tests and 2 doctests, including 19 memory-owner tests and 5 core unit tests. Formatting, workspace all-target Clippy and rustdoc with warnings denied passed. No production behavior was changed when these review tests were imported. Independent snapshot review remains separate from the user's required external review.
