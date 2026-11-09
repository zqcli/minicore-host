# Compose Persistent Owners

Status: implemented and integration-tested for this task scope; not full first-version completion.

## Scope

Compose existing real object storage and journal with Host -> Agent -> SessionController, preserving the same Runtime path and queues as memory owners. Support stable Host identity, fresh incarnation, hierarchical discovery, lazy history loading, real creation receipts, accepted-work ownership, persisted configuration and close/default ordering.

HostSettings is only currently supported identity metadata (name). ResourceResolver remains explicit application assembly. Full Host resource/network/log configuration, reload/shutdown, canonical service coverage, DSH, transport, interaction and compaction are outside this task.

## Verification and provenance

Commands and commit checkpoints are recorded below when run. Earlier local validation used an ignored Cargo patch while the Runtime extension was not yet published. Runtime is now published as `18a6ac9b82d003330cd870275efca793cdcc9a96`, and the parent integration has pinned that exact remote revision. This worktree's local validation still uses its existing patch: local Runtime HEAD `4f976b41f2381e2b09f3383aa66318b7aee2a8b3` and published `18a6ac9b82d003330cd870275efca793cdcc9a96` were independently compared and have the identical tree `46a3dc03a8ec0d89fc18ac39e9924eb1799ec45b`. No Runtime copy, bundle, absolute dependency path, root manifest or lock change is committed here. Remote-pin and clean-source verification belong to the parent integration; the historical local checks are not relabeled as clean-clone checks.

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

## Confirmed metadata and lifecycle checkpoint

- Persistent Agent/Session configuration and default changes retain the original candidate and confirmed baseline in the direct owner. Their asynchronous workers survive cancelled waiters and connection revocation. Effective values and versions publish only after storage confirms the matching record; a failed save is queryable and cannot be replaced by another candidate. Explicit reconciliation verifies the actual file against the original baseline or exact candidate before completing that same save.
- Session close first fences reception/execution. Its Agent confirms clearing the durable default before the Session can save and publish `closed`. Failures remain visible as `closing` with the old effective values; close does not erase history, invent a new default, or change a Turn's outcome. A memory default is runtime-only and is not restored after restart.
- Resume of an unloaded persistent Session now validates real history even without queued inputs; failed loading does not clear explicit pause. Explicit load is owner-held and does not replace retained context with an unvalidated prefix. Known line/byte positions are exposed through the registered history-load fault details.
- Reload reopens the actual journal only after pending save work has settled, under the backend's disk-only lock. This avoids treating `loaded_records()`'s initial baseline as a live cache and protects original failed save ranges. Real write -> unload -> reload -> Runtime tests verify complete same-process context.
- Added the review regressions for unsafe child-directory isolation, closed restart behavior, active persistent root leases, real identity-change save failures preserving completed execution outcome, and fresh incarnation/Load/Turn with saved context. The child-directory test uses storage's independent enumeration/load boundary.

Commands at this checkpoint: `cargo fmt --all`; `cargo test -p minicore-host-core --offline` (42 passed: 4 unit, 22 persistent owners, 15 memory owners, 1 real HTTP integration); `cargo clippy -p minicore-host-core --all-targets --offline -- -D warnings` (passed). `RUSTDOCFLAGS="-D warnings" cargo doc -p minicore-host-core --no-deps --offline` also passed. A later pause wins over an earlier accepted resume while its real history read is pending. A blocking-pool scheduling fixture proves both Agent and Session metadata saves continue after their accepted waiters are aborted, then the real Runtime consumes the confirmed configuration.

Still outside the implemented control surface: controlled Host shutdown, full Host execution-resource configuration/reload, automatic memory-entity reclamation and phased directory deletion, Runtime history-save reconciliation exposed through the owner API, canonical service/transport coverage, DSH, interaction and compaction. These are not reported as completed by the metadata APIs. A separately reproduced concurrent creation bug involving reused caller operation IDs is tracked for the next focused correction.

Committed core checkpoints before this change: `fc28786` (identity/discovery), `dddd347` (accepted creation), `f703977` (memory-descendant root lease). Storage integration prerequisites are the independently implemented `2366ff2`, `bf671a9`, `e24bcc0`, and `5746c2d`; their local cherry-pick SHAs are not new core work.

## Independent creation and worker retirement correction

- Each accepted Agent/Session creation now owns its receipt and task under its newly allocated object identity. Reused `operation_id` values do not merge, replace or reject invocations. Plural queries return every matching receipt; singular queries report ambiguity rather than selecting another invocation's state. Both deterministic concurrent reuse regressions preserve the actual Agent/Session/message ownership.
- Creation and metadata completion channels now send their final result only after removing task registration and releasing the worker's owner reference. This closes the observed empty-creation return/drop/reopen race without releasing an active Runtime's independent root lease. Tests perform immediate reopen after 72 empty creations across M/M, P/M and P/P and after 24 metadata workflows, with no sleeps or reopen retries.
- A completed Turn snapshot is not a Host shutdown or worker-join promise. Restart tests that first run a Turn now observe actual directory-lease release before opening a new instance. They fail on unexpected storage faults or a lease that does not release; elapsed time does not stand in for cleanup.
- Included the review checks for abandoned standalone Session creation, stop waiting for accepted creation, accepted configuration-version selection, 16 independent concurrent persistent creations, and async memory-child root lifetime. Configuration-version isolation is exercised on an Active persistent Agent; metadata mutation is explicitly rejected while initialization or stop is still underway.

Verification: full core tests passed (51 total, including 31 persistent owners), persistent-owner suite passed 25 consecutive runs, formatting and core all-target Clippy with warnings denied passed, and core rustdoc with warnings denied passed. Workspace tests (172 unit/integration cases plus 2 doc tests), workspace Clippy and workspace rustdoc also passed during this checkpoint. The previously completed metadata checkpoint is `8277546`.

Remaining review corrections are tracked separately: a stop aggregate must wake when a previously checked child later finishes metadata work, and `agent.start` must not activate an unfinished Creating Agent. Concurrent parent/child metadata replacement also exposed a safe but spurious storage validation failure; storage is reviewing that independent boundary. No result here claims these pending findings are fixed.

## Stop aggregation and initialization fencing correction

- Each Session receives only an Arc-backed parent notification token when its Agent constructs it. Session state publication wakes the parent's existing stop waiter, including completion of metadata/load work accepted after that child was checked. The child receives no Agent reference, business-state access, cross-owner lock or mutation shortcut; production stopping has no polling loop or timer-based workaround.
- `agent.start` now accepts only Active (no-op) or Stopped. Creating, Stopping and retirement states cannot be promoted to Active by a start call. Only the initialization owner commits Creating -> Active after its work finishes.
- Added independent review regressions for the late-child metadata completion wake and start during gated initial creation. Also included actual metadata/close/stop cancellation ownership, pending-load close retaining the root through durable closure, and a confirmed configuration update leaving the current real Runtime Turn bound to its old version while the next Turn uses the new version.

Verification: 56 core tests (36 persistent owners) passed; the five metadata/lifecycle review tests passed 25 consecutive runs; formatting, core all-target Clippy with warnings denied, and core rustdoc with warnings denied passed. The prior creation/retirement correction is `d7fe459`. The storage-level concurrent metadata replacement finding remains separate and pending its dedicated fix and regression.

## Close dependency scope correction

A real-file regression found that a retained failed Agent config save could unnecessarily block closing a healthy nondefault Session even after the shared storage path was usable again. The Agent now waits for its metadata only when the current default or the original pending candidate actually names the child being closed. A pending candidate that would make that child the default still fences its durable close until the same candidate is reconciled and the default is cleared. Unrelated Agent candidates remain retained and queryable; this change neither discards them nor retries them automatically.

Both opposing default-reference cases pass, together with the review's root-release/no-owner-cycle check across sync memory, async M/M, P/M, P/P and discovered persistent Session construction. Full core tests: 59 passed (39 persistent owners). Formatting, core all-target Clippy and core rustdoc with warnings denied passed. The prior lifecycle correction is `94e6029`.


## Final storage integration and handoff

- Integrated the independent storage correction `c2de8f1` (local cherry-pick `e3416fd`). Its per-file metadata read/publication coordination removes the reproduced legitimate parent/default-child replace race without weakening external replacement checks or adding an owner/global I/O lock.
- Retained the original concurrent case as `concurrent_parent_child_metadata_updates_keep_confirmed_records`: real Agent and default Session updates run together, both confirmed records are checked, then an immediate fresh Host reads the saved versions. This test is distinct from sequential worker-retirement tests.
- Added `crates/host-core/README.md` describing actual acceptance/completion boundaries, queryable failures, source/version selection, creation correlation semantics and remaining scope. The memory default-target precheck is also outside the access guard; the Agent's closing-set check still decides publication at its own boundary.
- Final local checks passed: `cargo fmt --all --check`; `cargo test --workspace --offline` (190 unit/integration cases plus 2 doc tests; core 60 = 4 unit + 40 persistent owners + 15 memory owners + 1 HTTP integration); `cargo clippy --workspace --all-targets --offline -- -D warnings`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --offline`. The complete 40-test persistent-owner suite passed 25 consecutive runs after the storage correction. No live-provider, TUI, clean-source or remote-pin test is claimed from this locally patched worktree.

Core commits before this handoff: `fc28786`, `dddd347`, `f703977`, `8277546`, `d7fe459`, `94e6029`, `2e65e94`. The parent owns integration-only documents, dependency pin/lock updates and any remote publication. No remote was pushed and no main branch was merged by this task.

The limitations above remain real: especially controlled Host shutdown, complete Host resource configuration/reload, automatic memory-entity reclamation and directory deletion, owner-level history-save continuation, and the complete canonical/transport/tool/interaction surfaces. Scoped persistent-owner completion does not mean the entire first edition is finished.
