# Task 033: Retain actual tool cleanup under the public Turn

The separate generic [terminal claim fix](CLAIM.md) preserves cancellation/failure ordering before history I/O and does not depend on the local-tool producer or registry.

## Scope and fixed inputs

- Base: `04ba5bc07a7d9bb73eabb143839fbaeb264a0cea`; topic `feat/turn-tool-cleanup`.
- SPEC v4.93 is unchanged: SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Runtime remains actual Git `69faf9196991524b5549499d6208a1eaf567bf56`, exactly `0.10.0`. No Runtime, TUI, dependency or manifest change in this core checkpoint.
- SPEC §§2/5/7/9/11 require actual execution ownership. Runtime cancellation can drop a `ToolFuture`; Runtime join is not proof that a tool's child process or already-started filesystem write has returned.

## Implementation

`PreparedTurnBinding::register_tool_cleanup` accepts trusted tool bundle owners during candidate preparation. The candidate capability is still inactive. Activation atomically seals registration and transfers the registered handles to the actual `ActiveTurn`; failed preparation publishes no Turn and consumes no input. Producers must use the existing admitted Turn capability before starting work.

The retained Session driver calls `TurnToolCleanup::finish` once after the public Turn claims its terminal outcome, including cancellation, force, startup failure and failed original history saves. Internal Loop continuation does not close a tool bundle. The driver and accepted original-save continuation retain ownership independently of a caller's wait or observations.

The cleanup interface returns `Ok(Vec<Fault>)` only after all actual workers are joined; the vector contains independent diagnostics. `Err(Fault)` or a panic means cleanup is unconfirmed, keeps the Turn finalizing, retains cleanup handles and prevents finished events, memory reclamation and directory retirement. No automatic cleanup retry or tool rerun is added.

`TurnInfo.cleanup_faults` and its state projection expose cleanup issues independently of the claimed business outcome/primary fault and persistence fault. These fields are diagnostics, not cleanup evidence. An unconfirmed cleanup fault remains a shutdown blocker even when the original history save is later confirmed.

## Validation

The eight cases in `crates/host-core/tests/task033_tool_cleanup.rs` run real Host owners, real Runtime, real temporary files and a gated `/bin/sh` child with an actual stdin write and `wait`. Only model responses and deliberate cleanup/save failures are fixtures. They require no public subscription.

- Cancellation drops Runtime's tool future before the real process returns; finalizing remains until actual wait.
- Force shutdown and an abandoned caller wait retain the persistent directory guard.
- Memory close cannot reclaim the Session while the real tool worker is running.
- Failed preparation retains queued input and never runs business tools or cleanup.
- Multiple internal Loops share one cleanup owner; separate Turns can reuse the upstream call ID without sharing ownership.
- A failed Runtime outcome and a confirmed cleanup diagnostic remain separate.
- A history identity failure still runs tool cleanup; explicit original-save continuation does not repeat cleanup or business execution.
- Unconfirmed cleanup and caught cleanup panic do not publish false finished or successful shutdown.

Focused command: `cargo test -p minicore-host-core --test task033_tool_cleanup --locked` passed 8/8. Initial compilation exposed a fixture typo, a nonexistent fixture `HostSettings::default`, and a mutex guard whose lexical scope crossed an await; all were corrected. The first aggregate lint run rejected the prepared cleanup field's type complexity; a local handle-list alias replaced that expression. A test fixture initially supplied a mismatched tool-call argument ID in its second internal Loop; it now uses the actual declared ID.

Aggregate validation passed on the final core topic tree:

- `./scripts/check.sh`: whitespace, rustfmt, all-target strict Clippy, all workspace tests, three doctests and strict rustdoc passed.
- `cargo test --workspace --all-targets --locked -- --list`: **502 unique target/test-name cases**, excluding nested storage subprocess output.
- `cargo metadata --locked --offline --format-version 1`: sole Runtime source remains the real Git revision/version above.
- `git diff --check` and the SPEC hash check passed.

The independent build target is `/workspace/shared/minicore-host-tool-owner-target`; Rust 1.90.0, two Cargo jobs, no debug info/incremental build, shared provisioned Cargo/Rustup homes and the project temporary root were used. Aggregate output and the unique-test list were retained in `/workspace/shared/task033-check.log` and `/workspace/shared/task033-test-list.log`.

## Remaining integration

This checkpoint establishes the actual core owner boundary; it does not yet enable ordinary production tools. Task 032's `host-local-tools` has its own genuine worker collector. Static registry wiring, an owner-held output lease that survives Runtime cancellation, actual command/edit recording, durable edit-journal storage/query and the production resource sink still need integration. The current task-local `ToolOutputSink` expires when the original ToolFuture drops and must not be presented as sufficient for owned cleanup output. No temporary/no-op sink is claimed as durable production capture.

No remote publication, live-provider request or TUI validation is performed by this task.
