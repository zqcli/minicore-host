# Task 071: Recover owner discovery

## Scope and source evidence

- This is fresh implementation and revalidation, not restoration of original bytes. Starting commit: `7cc0bd2b399c3ea1320647e13d87bf57376934a5`, tree `ba1c10a7b15bad746761272d7b87a564dfb29efa` (verified source-index-13). No comparison repository is treated as a behavior baseline.
- Worktree: `/workspace/shared/minicore-host-discovery-recovery`; branch: `task/071-owner-discovery-recovery`. Requested worker configuration: Astra, max reasoning. Effective runtime configuration is not independently verified.
- Prior task 057 source `9874a4b`, final documentation `38dd5a6`, and integrated range `9a6b9b3` through `8969f16` remain historical identifiers only. Full original patches and review artifacts have not been recovered. Historical test counts are not new validation.
- SPEC v4.93 remains byte-identical: SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`. Runtime, manifests, dependencies, Cargo.lock and TUI are unchanged. No push or merge into main was performed.

## Fixed implementation checkpoints

- Dependency `8c5c77790c5bcb57dae47839d26919f0ac4d0f14` is the local cherry-pick of task 070's `607f5204a28aaf680d6d0a4fa6fcfcce435fcbd6`: publish Session state when ToolStarted changes a retained ToolCall to Running. The coordinator already integrated that source and must not replay it twice.
- `59629af557f915eb7975426ca4249fff15440ebb` — `feat(core): expose owner discovery summaries`. Public DTOs, owner projection, state/end-reason plumbing, canonical transport routes and existing typed consumer updates are atomic.
- `a81fcef07b86e5f32261d2ceb3d87a76c5c549e0` — `test(core): verify owner discovery and cleanup`. Five real Host/Runtime/storage cases.
- `0c9719b711828b733c886009738d411b56497985` — `test(core): verify direct work publication`. Three direct-owner blocked-work cases.
- `de7571783aec423a04dcc9550aef54927ac228bc` — `test(query): verify discovery across adapters`. Two direct/stdio cases and one actual bound CoreTurnPort inspection case.
- `74350cf514de8b8e8ff182050f203cfea3232bce` — `perf(tools): avoid unrequested Agent config clones`. Default Agent inspect uses the same small owner summary; only include_config=true reads Agent configuration.

**Production files are frozen at `74350cf514de8b8e8ff182050f203cfea3232bce`.** Subsequent task-ledger changes do not reopen production-file ownership.

## Implemented contract

- SPEC §§1, 8, 9: Host info reports its actual name/version, supported storage assembly, registered reload preparer, configured model/tool counts and revision, open storage, retained execution resources and configured network authentication. Authentication does not claim a running external listener or provider health. Process logging health belongs to the separate diagnostics integration and is not fabricated here.
- Agent and Session list DTOs contain selected identity, metadata and current work fields. They do not clone effective configuration, Role, history, output bodies or execution resources. Agent summaries combine direct Session summaries after releasing the Agent lock. Agent deletion is projected from its own DataDeletion, not its cascade report.
- AgentListRequest and SessionListRequest use exclusive lexical typed-ID cursors. None limit returns all remaining entries; zero is a normal validation fault. Session mode/lifecycle filtering precedes page size. There is no capacity quota or cursor registry.
- Canonical agent.list/session.list routes return pages. Existing all-items helpers call the same implementations. agent.get returns a freshly constructed AgentDetails with flattened AgentInfo and direct Session summaries; no summary copy is retained in Agent state.
- SPEC §§3, 5, 9: Session state and lists expose actual model phase, unfinished ordinary ToolCalls, preparation, accepted/unconfirmed history and metadata saves, active compaction association, actual tool cleanup handles, scope cleanup and result handoff. Finished compaction diagnostics are not active work; an empty cleanup bundle is not pending work. Independent child execution is not a running ordinary tool.
- ToolStarted and the transition to handoff_pending publish the authoritative Session snapshot before a blocked caller can observe stale idle work. No execution, save, cancellation or cleanup ordering is replaced.
- The owner fixes end_reason at Open -> Closing using the actual existing close cause, retains it through close saving and restores the stored SessionClosure reason verbatim. Turn completion and controlled Host shutdown do not invent a Session closure reason. No history scan or new terminal-result cache is used.

## Observed retirement race

A deterministic metadata-save/deletion test found that Agent enumeration can copy a child just before that child is reclaimed. Current list projection omits only a child whose actual state.reclaimed is already true. Permission, storage and internal faults still propagate; they are not converted to empty lists. Persistent Closed records that remain present are still listed. Direct Session lookup keeps its original not-found behavior. This is a narrow current-list rule, not a retry or fallback framework.

## Validation and red/green evidence

Build environment: `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, Cargo on PATH, `TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, `CARGO_TARGET_DIR=/workspace/shared/minicore-host-target071`. All Cargo invocations used `--locked`; Cargo.lock was not regenerated.

- Starting-source sanity check: `cargo test -p minicore-host-core --test runtime_owners --locked`: **28 passed**. Log `/tmp/task071-baseline.log`. This is not validation of lost task 057.
- Behavioral red: with the new owned-work projection but before the transferred ToolStarted publication, `model_tool_and_cleanup_work_follow_real_runtime_and_published_state` entered the actual gated tool but its state subscription never reported Running. **3 passed, 1 failed**, timeout at the state-publication gate. Log `/tmp/task071-discovery2.log`. The exact transferred one-line hook made this test green.
- Final focused checks on `74350cf`: `cargo test -p minicore-host-core --lib task071_ --locked` (**3**), `cargo test -p minicore-host-core --test task071_discovery --locked` (**5**), `cargo test -p minicore-host-tools --test task071_agent_inspect --locked` (**1**), and `cargo test -p minicore-host-transport --test task071_discovery --locked` (**2**): **11 new cases passed**. These cover blocked model/tool/save/metadata/compaction/handoff/scope cleanup, actual closure/reopen, resource release while query values remain held, own deletion status, config omission, lexical/None pagination, filtering before paging, and direct/stdio/Agent inspect projections. Log `/tmp/task071-final-focused.log`.
- `cargo fmt --all -- --check`: **passed**.
- `cargo clippy -p minicore-host-core -p minicore-host-tools -p minicore-host-transport --all-targets --locked -- -D warnings`: **passed**. Log `/tmp/task071-clippy.log`.
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core -p minicore-host-tools -p minicore-host-transport --no-deps --locked`: **passed**. The same packages' `cargo test ... --doc --locked`: **1 transport doctest passed**, zero doctests in core/tools. Log `/tmp/task071-final-focused.log`.
- Final `cargo test -p minicore-host-core -p minicore-host-tools -p minicore-host-transport --all-targets --locked`: **478 passed, 0 failed**, across 59 test binaries. Log `/tmp/task071-affected-tests-final.log`.
- Earlier affected-crate run: untouched `task054_query_contracts::raw_result_and_wait_keep_complete_and_partial_indexes_across_internal_loops` failed at its server teardown (`task054_query_contracts.rs:503`, BrokenPipe). It passed both a standalone rerun and the final full affected-crate run. No fix to that fixture is claimed; task 070 owns its investigation. Logs `/tmp/task071-affected-tests.log`, `/tmp/task071-query-rerun.log`.
- `cargo test --workspace --all-targets --locked -- --list`: **passed**, listing **840 unique test cases across 96 test binaries** on the frozen source branch. Counted `(suite, test name)` identities match the listed summaries, with no duplicate identities. This is a complete compile/inventory check, not execution of the full workspace suite. Log `/tmp/task071-workspace-list.log`.

## Integration handoff and limits

The coordinator has the five task commits above and the separate dependency. Preserve task 066's credential getter/PreparedTurnBinding changes and ToolEnvironmentExclusions export, task 070's configuration-free Turn inspect, and the shared ToolStarted hook when composing the stage. Diagnostics may extend Host info without replacing these discovery fields. Production-file ownership is released for that integration.

The source-branch checks above are not an independent review or a complete combined-stage workspace gate. Those remain with the coordinator. No live external provider or TUI validation is claimed; all scripted models/tools and gates are named test fixtures. The original missing patch/review evidence remains missing.
