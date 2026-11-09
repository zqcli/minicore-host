# Task 046: Read owner-held Session context

## Scope and source

- Base: `76a7bfbe7f70d7f707d48cc38190829d14c137f2`, on the local `feat/session-context` worktree. No push, main merge, TUI change, dependency change or transport edit.
- Implements the in-process `session.context` query required by SPEC v4.93 §8, with the direct ownership, configuration, history and observation contracts from §§2, 4, 5 and 9. SPEC remains byte-for-byte unchanged: SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739` was read as code reference, specifically `src/sessions/budget.rs`, `src/sessions/types.rs` and compaction context logic. Its history estimates, context budgets, recovery observations and old execution limits are not a Host behavior baseline.
- Uses the existing Session raw history, committed `CompactionProjection`, `CompactionActivity`, save work and Runtime `Usage`. No second compaction system, token cap, prompt-preparation query, history cache, retained output service or resource-preparation mode was added.
- SPEC line 1426 assigns retained/stored content reads to `session.history.read`; line 1427 assigns context usage, compaction and execution information to `session.context`; line 1457 permits module-specific auxiliary query fields. Accordingly, a cold `history: null` is an unknown owner fact, not a fabricated empty/zero context or a claim that the file was validated. This scope decision does not close the separate history-read risk below.

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

## Exact functional source and final verification

- Functional source: `a412d7fbac03c4e810d38ae7b6f34a8be279b0bc` (`feat(core): expose owner-held session context facts`). All following checks ran against that fixed hash with a clean worktree, before this report-only follow-up commit.
- `./scripts/check.sh`: passed on its first final-source run, including repository whitespace checks, rustfmt, workspace all-target Clippy with warnings denied, every unit/integration target, four doctests and strict rustdoc. No failing full gate was repeatedly retried to obtain a green result. Log: `/workspace/shared/minicore-host-session-context-artifacts/a412d7f-full-gate.log`.
- `cargo test -p minicore-host-core --test session_context --locked` and `cargo test -p minicore-host-core --lib session::context::tests --locked`: all 10 public-Host cases and both gated owner cases passed three consecutive repetitions after the full gate. Log: `a412d7f-focused-repeats.log` in the same artifact directory.
- `cargo test --workspace --all-targets --locked -- --list`: **662 listed, 662 unique target/test-name cases**, across 67 targets containing tests. Nested storage subprocess execution output is not an additional case. Inventory: `a412d7f-test-list.log`; machine-readable summary: `a412d7f-verification-summary.json`.
- `cargo metadata --format-version 1 --locked` confirms the sole Runtime dependency is version `0.10.0` at `git+https://github.com/zqcli/minicore-runtime?rev=69faf9196991524b5549499d6208a1eaf567bf56#69faf9196991524b5549499d6208a1eaf567bf56`, with no path patch or regenerated lock. Metadata: `a412d7f-metadata.json`.
- `SPEC.md` SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`; `Cargo.lock` SHA-256 remains `9084c32031f24813b50bf2c2181c7f055ffc86650ef1690b6c7bf7865a143e89`.
- The untouched TUI reference checkout remains `3beeb1d8a525e58ce6352e05061f242c5580e248`. Its historical Runtime pin is not this Host's current Runtime pin; no joint TUI integration is claimed.
- Independent review and canonical transport integration remain the parent/integration owner's next steps. No source fix outside this task, remote publication or main merge was performed.

Reproduction environment for every final-source command:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-session-context-target
```

## Independent review correction: public compaction association

Independent review of `a412d7fbac03c4e810d38ae7b6f34a8be279b0bc` reproduced that the reused public `CompactionActivity` exposed `CompactionSource::Automatic.loop_id/request_index`. That contradicted the public Turn/continuous `response_index` boundary in SPEC lines 481, 566 and 568. The original reviewer source and red logs remain unchanged under `/workspace/shared/session-context-review-a412d7f/`.

The first correction changes the public `CompactionSource` itself, so context, session.state and observation share the same representation. Automatic admission validates the actual private Turn/Load/Loop binding under the Session owner lock, then captures `active.response_base + request_index` as the public `response_index`. Internal `CompactionBinding` and model-call validation retain their original Loop identity. Manual operation association and independently numbered utility usage are unchanged. The existing resources compaction test only changes its public field assertion.

A permanent real-Runtime Host regression runs automatic compaction during a second internal Loop in the same Turn. It verifies public response index 1 in context, direct state and subscription snapshot, absent internal IDs, two actual distinct raw Loop IDs with internal request index 0, and utility request index 0. The original independent internal-ID probe was appended unchanged for verification and removed afterward.

Pre-commit verification: the two automatic context tests, all 15 core compaction owner tests, all three resources tests selected by `compaction`, the original independent internal-ID probe, and all-target strict Clippy for core/resources passed. Logs are `review-fix-compaction-{source-focused,original-probe,owner-suite,resource-suite,clippy}.log` under the task artifact directory. The separately reproduced accepted-history/save-registration race remains open for the next correction; this first commit does not claim to fix it.

## Independent review correction: atomic history acceptance and save facts

The public compaction correction is fixed at `01e5946` (`fix(core): map compaction to public response identity`). The second reviewed defect was a genuine owner-observation window in the pre-existing history producer: complete native items and completion activity became readable before a second lock acquisition registered their original save. The reviewer observed 70 raw items with accepted journal position 68 and neither a pending nor a running save. No query field is removed or changed to unknown to hide that mismatch.

`session/history.rs` now separates synchronous owner registration from lock-free append/settlement. Complete items, partial content and terminal acceptance each register their original save binding, accepted journal position, running work and applicable pending persistence in the same owner boundary as the corresponding content/cursor/acceptance flags. Publication follows that atomic change. Actual backend append and settlement remain outside the lock; original save ranges, same-Turn continuation mapping, suffix deduplication, reconciliation, final outcome versus save fault, tool cleanup and cancellation/join ownership are retained. Memory still reports `NotApplicable` and uses its existing non-writing backend. There are no extra fields, production fixture hooks, caches or services.

Permanent regressions added for this correction:

- Two public Host tests use gated real Runtime continuations, for persistent and memory Sessions. The first Session and Turn states immediately following each accepted `ModelFinished` must already describe the corresponding save. This is deterministic event-boundary coverage, not a scheduler lottery. Replacing only `history.rs` with the old committed version makes both new cases fail at the missing-save assertion; restoring the correction makes both pass. The second request starts after the first save has settled, covering the old persisted-state window.
- A Session test gates each actual real-journal append for complete input, incomplete partial response and terminal record. At each gate it verifies accepted position, readable/partial/terminal facts, pending/running save, correct persistence and still-unwritten disk range. The final file contains exactly the original input/partial/failed-terminal sequence once, preserving business failure separately from successful persistence.

All four original independent reviewer probes were appended unchanged, run, then removed from the working test file. All passed, including 256 real same-Turn continuations with **77,945 concurrent context samples and no inconsistent snapshot**. The original reviewer probe and both original red logs were checked unchanged with SHA-256. Artifacts: `review-fix-original-probes.log`, `review-original-evidence.sha256`, and `review-fix-acceptance-old-source-red.log` in `/workspace/shared/minicore-host-session-context-artifacts/`. The permanent focused acceptance tests, all core all-target tests, formatting, whitespace and core all-target strict Clippy passed before this correction commit. Fixed-source full-gate/inventory results follow separately; the independent history.read lock risk and other owners' storage/diagnostics work remain outside this correction.
