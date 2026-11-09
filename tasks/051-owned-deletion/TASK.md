# Task 051: Delete explicitly owned data

## Frozen baseline and scope

- Base: `accc5ddd120444cabe90c4756f5efa2bb047392c`; isolated topic `task051-owned-deletion`.
- SPEC v4.93 is unchanged: SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Runtime stays exactly 0.10.0 at `69faf9196991524b5549499d6208a1eaf567bf56`; Cargo.lock remains pinned.
- Worker requested configuration: gpt-6-astra / max. No independent effective-runtime attestation is claimed.
- No push, main, TUI, storage implementation or context/history/compaction implementation changes.

## Request and receipt freeze

`session.delete` has `operation_id` and `close` (default false). Default admission requires closed and settled. `close:true` explicitly authorizes normal close and required settlement, then deletion of only that Session's owned data.

`agent.delete` has `operation_id` and `scope` (default `empty`). `empty` requires stopped with no retained Sessions. `sessions` contains an explicit list of direct Session IDs plus `close` (default false). The set must exactly cover the retained children at the Agent admission boundary. Unknown/unavailable children and in-flight creations cannot be silently omitted. `close:true` permits the required Agent stop and normal close of those Sessions; it never grants directory-wide recursion or deletion of collaboration descendants owned elsewhere.

The direct parent owns accepted deletion work through caller timeout/disconnect. The control receipt forms at the acceptance boundary after admission is fenced and work registered, not after the entire deletion. Receipts remain immutable. Existing Agent get and Session state expose the current direct owner's typed deletion state, explicitly requested scope and partial outcomes/faults. Successful removal returns normal not-found through the former child query. Session deletion receipts belong to Agent; Agent deletion receipts belong to Host, subject to their real owner lifetimes. Old child-owned receipts are not copied to an archive.

Current deletion facts separate pending/not-attempted work, physical deletion, memory reclamation and failures. Failures may follow already committed close/deletion steps and do not imply rollback or that all files remain. This is narrow state for the two deletion methods, not a generic operation or recovery platform.

## Exact specification boundaries

- Section 3, Agent lifecycle/default entry (lines 235–245): retirement fences, durable default clearing, stopped/no-child default deletion, explicit cascade.
- Section 3, Session lifecycle (lines 267–295): close is distinct from delete; real execution, tools, interactions, saves, result handoff and root-scope cleanup precede closed/release.
- Section 4, storage ownership (lines 313–330 and 420–424): direct recorded ownership, no implicit deletion, no recursive expansion, no removal of parent results or independent artifacts.
- Section 6 (lines 743–745, 977): typed owner facts, accepted work survives wait loss, formed receipts remain immutable; operation_id is correlation only.
- Section 8 (lines 1367, 1388): canonical agent.delete and session.delete methods.

## Reference reuse

Read minicore-agent reference `f36a87366d8e2fe0a1c8216463887902e471c739`, `src/agent.rs::delete_session` and `src/store.rs::delete_session`. Its worker-settlement lesson is suitable; its rename/recursive-directory removal is not this Host's behavior baseline. Reuse Host's actual owner task/receipt/retirement boundaries and existing `SessionStore::delete_closed` / `AgentStore::delete_empty` instead.

## Validation

Pending implementation and focused checks. Known aggregate storage/diagnostic lease corrections are independently owned and are not patched or rerun to hide failure here. Full composition gate belongs to the integration owner.
