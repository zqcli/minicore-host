# Task 033 follow-up: Preserve the first terminal claim

This follow-up changes only the generic Session claim path and tests. The two new failing-before-fix cases run actual Runtime rather than fabricated reports:

1. An interrupted model stream returns a real Failed report. A gate holds the subsequent original Partial save. Previously the Session still reported Running and accepted a cancellation despite already possessing the failed report.
2. A cancellation is first claimed by the Session while a real model stream waits. Its deliberately panicking destructor makes actual Runtime return its Internal Failed panic report. Previously the Host replaced its already-claimed cancellation with Failed.

A third gate confirms the ordinary cancellation-before-history-save-failure order: Runtime itself retains Cancelled and separately reports `history_failure`. That case already passed; the test does not manufacture a Failed report.

The Session now claims failed/cancelled reports under its existing short state lock before saving the returned records, and one `ActiveTurn::claim_terminal` preserves the first public outcome. A later execution fault after a claimed cancellation goes to `cleanup_faults`; original history faults remain separate. The completed-report path retains its existing same-Turn next-step continuation decision. Terminal history reason no longer substitutes a later failed-report diagnostic for an already-claimed cancellation.

The minimal added structure used by this fix is `TurnInfo.cleanup_faults` and its public state projection/initializers from generic core checkpoint `704c230`. The fix has no dependency on `7f7c280`, the local-tool crate, registry changes or new manifests.

### Existing Agent code reference

Read-only comparison used Agent source `f36a87366d8e2fe0a1c8216463887902e471c739`, not an implementation or compatibility baseline. `src/task.rs::OwnedTask` and `src/sessions/active_loop.rs::join_runtime_loop` confirm the useful existing design of retaining the real join owner independently of the waiter. The Host already has that driver ownership, so no second task wrapper was copied. Agent `active_loop.rs::cancel` rejects an already-finished Runtime handle, and `sessions/compact.rs::CompactionOperation::complete` preserves an earlier owner cancellation. The Host applies those ordering principles in its existing Session mutex, without copying Agent's atomic compaction lifecycle.

Agent uses LoopId as its Turn and can expose completion before auxiliary settlement; Host SPEC §§3/5/11 instead requires a public Turn spanning internal Loops and actual cleanup/necessary saving before Finished. Those old semantics and the Agent's result-catalog/budget machinery were not transplanted. The Host SPEC and current task requirements decide this fix.

Before-fix evidence: `/workspace/shared/task033-claim-before.log` shows two failing assertions (Running instead of Finalizing, Failed instead of Cancelled) and one passing history/cancel case. After-fix focused command `cargo test -p minicore-host-core --lib terminal_claim --locked` passes all three; evidence is `/workspace/shared/task033-claim-after.log`.

Full validation on the topic worktree passed `./scripts/check.sh`: formatting, strict all-target Clippy, workspace checks/tests, doc tests, and warnings-as-errors rustdoc. The workspace contains 538 distinct tests and 3 doc tests at this checkpoint; this count includes the separately paused local-tool work already present on the topic, not a claim that those tools have been enabled or accepted for production. Logs: `/workspace/shared/task033-claim-check.log` and `/workspace/shared/task033-claim-list.log`. SPEC and the actual Git Runtime pin remain unchanged.
