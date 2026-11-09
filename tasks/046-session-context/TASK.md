# Task 046: Read owner-held Session context

## Scope and source

- Base: `76a7bfbe7f70d7f707d48cc38190829d14c137f2`, on the local `feat/session-context` worktree. No push, main merge, TUI change, dependency change or transport edit.
- Implements the in-process `session.context` query required by SPEC v4.93 §8, with the direct ownership, configuration, history and observation contracts from §§2, 4, 5 and 9. SPEC remains byte-for-byte unchanged: SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739` was read as code reference, specifically `src/sessions/budget.rs`, `src/sessions/types.rs` and compaction context logic. Its history estimates, context budgets, recovery observations and old execution limits are not a Host behavior baseline.
- Uses the existing Session raw history, committed `CompactionProjection`, `CompactionActivity`, save work and Runtime `Usage`. No second compaction system, token cap, prompt-preparation query, history cache, retained output service or resource-preparation mode was added.

## Public contract and sources

- `Host::session_context(&CallerContext, &SessionRef) -> Result<SessionContextSnapshot, Fault>` is async for the public adapter, but its implementation is a pure read of the actual Session owner. Routing remains Host → Agent → SessionController; read-only callers may query and revoked/wrong owners fail normally.
- `SessionIdentity` is flattened once outside `data`. The snapshot includes controller revision, residency/LoadId, whether execution history is loaded, the committed Session config revision/model/compaction settings, queue counts, an optional active Turn binding, the existing current-or-last compaction activity, original-history-save pending/running/failure facts and a known blocker.
- The active Turn holds only small value facts captured from its selected Session config and actual prepared model descriptor before taking the owner lock. It exposes the original TurnId, LoadId, config revision, model reference/window and compaction settings. Session updates and Host reloads do not rewrite this value; internal continuations retain it. Role text, full SessionConfig, clients, tools and prompts are not duplicated or retained by this query.
- `history: null` means the owner has never accepted or loaded history. A cold file is not inspected or called empty. A real empty memory or loaded persistent context has `Some` history with zero native items and no complete response. Explicit load/resume owns JSONL validation. After unload, retained accepted context is still readable while `history_loaded` remains false until an explicit load succeeds. A failed reload retains the prior readable copy together with the actual blocker; it never makes that copy executable.
- `raw_items` counts complete native history items only. `effective_items` counts the actual committed summary plus the uncovered suffix. Queued and bound-but-unhanded input, partial model content, finish records and compaction markers are not fabricated as complete native items. `position` is the existing Session-local accepted journal-entry position, not a persistence watermark or public content cursor.
- The projection contains actual committed source position, covered raw count and summary. Pending/failed compaction candidates never appear as committed projections. The existing compaction activity separately carries utility usage and its before/after scheduling estimates (JSON request bytes divided by four), not exact provider token counts.
- `latest_response` identifies the latest complete raw assistant item using its raw item index and model, with its reported `Usage` and whether compaction covers that item. No internal LoopId/request_index is introduced as a public response identity. Absent usage fields remain absent, a later unknown response does not borrow old counts, and a provider-reported zero remains zero. This is a historical response's usage, not the size of today's context or an aggregate. No full-request estimate is manufactured without an actual prepared request.
- Querying does not load a Runner, open files, consume input, unpause, prepare resources/prompts, call a model/tool/utility, confirm/retry a save, publish state or retain a source body beyond the ordinary returned value. Owner/access checks and the returned snapshot are taken at the same short read boundary.

## Permanent regressions

Ten integration cases use the actual public Host entry point, real Runtime, real temporary history and explicitly named scripted models:

1. Real empty context, separate queued inputs, read-only access and no query-triggered preparation/execution.
2. Actual complete-response usage, absent fields, a later unknown response and genuine reported zero.
3. Queries and sends during a gated model call; the active Turn and same-Turn continuation retain the original config/model window after both Session update and Host reload, while the next Turn binds the new resources.
4. Cold unknown history becomes actual loaded content and remains readable across unload/reload without taking a query file lock.
5. Empty persistent history is unknown before load and known empty afterward.
6. Cold corruption is not pretended validated by a query; explicit load establishes the actual blocker.
7. Failed reload preserves readable prior content but not execution eligibility.
8. Real manual utility compaction, concurrent queries and queued send, committed summary/coverage, utility-versus-ordinary usage, unload/reload and cold restart without resurrecting runtime activity.
9. Real automatic compaction associated with the active Turn; repeated queries do not launch more utility work or count protected unhanded input.
10. Incomplete model output is not a complete history response; reclaimed memory is an ordinary not-found fault, never an empty success.

Two Session-level unit regressions wrap the real journal with an explicit test scheduling gate and injected pre-sync failure. They verify pending versus failed original Turn saves, readable bytes without save confirmation, and compaction candidates remaining unapplied until original-range reconciliation actually succeeds. These two cases intentionally exercise the Session directly, avoiding production Host/Agent test-only constructor hooks. All other new cases use public Host routing. No external provider or TUI execution is claimed.

## Verification

- Before the functional commit, the 10 public Host tests and both gated owner tests passed. Focused strict Clippy initially caught one unnecessary borrow in a test; it was removed rather than suppressed.
- Initial test work exposed a real read/load lock interaction in the initially considered cold file-reading query. The final query is pure owner state and contains no file-read path. The existing independent history-read entry point was probed separately, as described below; no regression is hidden by skipping or ignoring a permanent test.
- Exact functional-source hash, full gate, focused reruns, test inventory and dependency metadata are recorded in the follow-up verification section after those commands finish.

## Independent existing history-read risk

The existing `Host::history_read` cold-file path calls `SessionStore::read_history`, which takes a shared file lock. Concurrent `load_session` opens the journal with an exclusive nonblocking lock. A separate bounded public-API probe reproduced, on its first iteration, `history.read` failing with `SESSION_HISTORY_LOAD_FAILED: history already has an active writer` while the explicit load succeeded. Earlier cold-file context experiments showed the same contention; no such path remains in `session.context`.

Minimal reproduction: seed valid persistent history, open a fresh Host without loading the Session, start `history_read` with limit 1 and concurrently call `load_session`, then inspect the read result and owner load state. Probe source and exact observed log are in `/workspace/shared/minicore-host-session-context-artifacts/history-read-load-risk-probe.rs` and `history-read-load-risk-probe.log`. The snippet was temporarily appended to this task's integration fixture and run with `cargo test -p minicore-host-core --test session_context --locked history_read_load_race_probe -- --ignored --nocapture`, then removed. It is not a shipped ignored test or part of the test inventory. No modification to history.read, the backend, loading or storage is claimed; the integration owner tracks that independent scope.

## Integration and remaining work

The transport owner has the frozen Host method and DTO, including nullable history. This topic deliberately does not add the canonical RPC route or typed client method. The integration owner maintains PROJECT.md and performs independent review/integration. This task does not claim every catalog operation or the full product is complete.
