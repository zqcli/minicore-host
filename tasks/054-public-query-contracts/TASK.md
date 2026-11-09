# Correct retained public query contracts

## Scope and ownership

Base: `a748d20a6cbaf4c4ba8572619c696d3004e70440`, documentation over verified
code `70c17e7226bd90c7bb1e146ed141081f567e9d23` (761 unique tests, 4 doctests).
Topic: `topic/public-query-contracts`. The parent maintains aggregate PROJECT.md.
SPEC v4.93, Runtime `69faf9196991524b5549499d6208a1eaf567bf56`, Cargo.lock,
TUI and publication targets are unchanged. No push, main merge, extra canonical
method, lifecycle registry, compatibility shim or copied retention layer is added.
The requested Astra/max configuration cannot be independently attested here.

Independent audit source: `f7ee4a1f8650ab2fb0aced250ea43e89932b1b93`.
Preserved evidence: `/workspace/shared/method-contract-audit-f7ee4a1/REPORT.md`,
`probe-red.log` and `method_contract_probe.rs` (SHA-256
`84ccd4b3d8bd5bbab9fc18f605b9fc783917ba0e01950dc6c3fcc8c030b09b16`).
The original probe checkout was not edited. Its three reproduced query tests
are carried into a permanent transport regression; the separate schema-only
SessionSnapshot decoder concern is not expanded into this fix.

## Changes

- Root bootstrap `d4fda9589265d1f483afe9edc8aa76059f45e4ca` stores the exact
  accepted `preparation.0` SessionConfig on TurnInfo. Existing completed Turn
  retention carries it forward; queries never read today's Session config or
  re-resolve resources to reconstruct old bindings.
- Detailed turn.get returns the original configuration and source config
  revision with the existing state facts. TurnStateData, Session state and
  observation baselines/events remain lightweight and config-free.
- Canonical message.get, turn.list, turn.result and turn.wait use explicit
  identity/data projections shared by in-process, stdio and WSS dispatch.
  MessageId and applicable Turn/Load identities are outer fields; Host and
  Session identities are not duplicated within data. Completion order remains
  unchanged. Missing/reclaimed owners still return their actual Fault.
- Result/wait preserve retained immutable outcome and persistence facts,
  complete native history, and separate unfinished partials. Public history
  reuses HistoryItemData and ModelPartialData; Runtime loop/request/replay
  metadata is excluded. Complete response indexes come from the original
  accepted output projection, joined in native complete-response order. Tool
  records use the corresponding original native Loop/request pair. Partial
  indexes are the already retained public indexes, never a take count.
- No native history or raw internal delegation handoff is rewritten. Queries
  only project values owned by the original Session. Canonical wait retains
  the existing weak-owner state subscription and polls only a completion fact.
- Existing transport assertions are mechanically migrated to explicit snapshot
  fields. Observation parity still compares the same TurnStateData. No test
  is removed and no behavior assertion is weakened to preserve the old shape.

## Validation in progress

Evidence: `/workspace/shared/public-query-validation/`; environment in `env.sh`.
Independent target, Rust 1.90.0, locked dependencies, jobs=2, no incremental build.

- Initial production `cargo check -p minicore-host-transport --locked`: passed.
- Transport-test compile migration exposed four missed old field paths; exact
  compiler failures are preserved in `test-migration-check*.log`. Corrected
  test-only paths pass `test-migration-check-4.log`.
- The new pipe parity test exposed accidental live runner_revision in finished
  result snapshots (30 versus 31 during normal driver settlement). Result info
  now projects only retained immutable terminal facts; get/list keep actual
  state revisions. The original assertion failure is preserved.
- New fixture failures are retained: an empty canonical model response without
  replay is invalid in Runtime, a complete business outcome remains Completed
  despite a save failure, and reclaiming the last memory Session may also
  reclaim its Agent. Tests now use an actual replay-only empty response,
  assert outcome separately from persistence, and retain a second Session to
  make the exact direct-owner missing-source assertion deterministic.
- Focused core tests: 2 passed (`focused-core-3.log`), including actual multiple
  Runtime Loops, tool-only and empty complete responses, partial index 2 after
  complete indexes 0/1, later Turn binding/index reset, and real journal failure
  reconciliation retaining the original byte prefix.
- Focused transport tests: 4 passed (`focused-transport-3.log`), including the
  three original query probes and real OS-pipe/typed parity, read-only and new
  operator identity, state/config separation, timeout without cancellation,
  wrong-target failures and actual memory reclamation.
- Full transport suite and final fixed-source aggregate gate are pending.
  Independent review and parent integration remain separate.

## First fixed-source aggregate attempt

Implementation `daf24b2` and transport regressions `63a5dbb` are separately
reviewable commits after the root bootstrap. The whole transport test suite
passed (`all-transport.log`). Aggregate `./scripts/check.sh` at exact
`63a5dbb22249fb96e4e4da1c656e267cfbdfcba3` stopped in Clippy compilation:
`crates/host-app/tests/process.rs` still used three old typed query field paths.
The failure is preserved in `full-gate-63a5dbb.log`; later aggregate stages did
not run. The following test-only correction migrates those paths without
changing the real subprocess/TLS scenario or assertions. It also adds direct
raw stdio result/wait assertions for a real same-Turn second-Loop partial, so
Runtime metadata exclusion is checked on the actual wire as well as core
serialization. The aggregate will be rerun only after this concrete correction.
