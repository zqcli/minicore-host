# In-process owners

`minicore-host-core` composes the real Host -> Agent -> SessionController ownership path with Runtime. Memory and persistent Sessions use the same queues, configuration binding, Runtime loop and result handling. A connection grants access; it never replaces the target's direct owner.

This crate implements the in-process business owners; transport servers and executable assembly live in `host-transport` and `host-app`. Runtime integration uses the published Git revision pinned by the workspace; no local dependency patch is needed. Implemented APIs do not by themselves establish live-provider or TUI acceptance.

## Persistent initialization

`Host::open_persistent(path, initial_settings, resources).await` acquires the exclusive data root before constructing executable owners. A new root saves its identity; an existing root uses its saved Host ID and supported settings. Each open receives a fresh incarnation. The name-only `HostSettings` is an embedding construction convenience, converted to the single complete `HostConfig` disk shape. Production composition uses `Host::open_configured` and the `minicore-host-resources` builder. Config queries, immutable resource catalogs, and accepted reload publication belong to Host; raw keys and execution handles are never stored in the record.

The Host reads direct Agents. Each Agent reads its own metadata and initializes its direct Session metadata independently. History remains unloaded until the Session's load/resume/execution path validates it. Stored execution handles, queues, pauses, Load IDs, active Turns and tool processes are never reconstructed. Closed Sessions remain closed.

Malformed or unsafe child entries remain associated with their actual IDs through `agent_load_failures` or `session_load_failures`. They do not silently become empty objects or hide healthy siblings. An unavailable default remains unavailable; it is not redirected or replaced automatically. Unsupported records/settings are rejected without overwriting saved values with defaults.

P/P, P/M and M/M are supported. M/P is rejected. Memory descendants of a persistent Host retain its root lease during accepted work but create no identity/history files of their own.

## Acceptance and completion

- `create_agent_async` and `create_session_async` register owned work before disk I/O. Aborting the caller's wait or revoking its connection does not cancel that work. The returned receipt describes actual completed creation steps, partial failure, and initial-message delivery facts. An accepted initial message may still have an independently running Turn.
- Every creation invocation has independent work state. `operation_id` is correlation data, not an idempotency or replay key. `creation_receipts` and `session_creation_receipts` return all matching invocations; the singular queries report ambiguity when a caller reuses an ID. Published object/message IDs are never borrowed from another invocation.
- `update_agent_async`, `update_session_async` and `set_default_session_async` await confirmed metadata saves before publishing effective values, versions or defaults. Queryable `metadata_saving`/`metadata_fault` fields expose unfinished or failed work. Agent metadata changes are accepted in Active/Stopped, not while initialization or stop is still running.
- `reconcile_agent_record` and `reconcile_session_record` explicitly continue the original failed metadata candidate. They check the actual record against the original baseline or exact candidate. A different file is not overwritten, and another candidate cannot replace the retained pending save. These narrow metadata helpers do not reconcile Runtime history saves. The public Session `resume` path also confirms its own retained original history and metadata saves, as described below.
- `load_session` accepts owned loading work. `resume` validates required unloaded history even with an empty inbox and clears allowed pause reasons only after successful loading. A later explicit pause wins over an earlier pending resume. Inspect the Session snapshot/subscription for loading or save-confirmation completion and its real blocking fault. A retained original closure save may finish Closing, but Closed never reopens.
- `close_session` fences reception and starts owned close work. The direct Agent confirms clearing its durable default before the Session saves and publishes `closed`. Failure leaves a visible `closing` state and the original save context. A memory default is represented as null on disk and is never recreated on restart.
- `shutdown` returns the canonical immediate actual strategy/lifecycle receipt, then owns graceful draining or force cancellation, joins, saves and resource release. Explicit force can upgrade graceful; repeated requests report current facts without restarting work or retrying saves. Paused backlog remains observable while draining.
- `stop_agent` fences new execution/creation and waits for its accepted work and children. Child state changes wake the parent's aggregate without cross-owner state access. `start_agent` resumes Stopped or is a no-op for Active; it cannot promote unfinished Creating/Stopping/retirement states.

The synchronous creation/configuration convenience entries support memory ownership. Persistent mutation uses the asynchronous entries; it never silently substitutes a memory backend. A completed creation or metadata waiter follows retirement of that temporary worker. A completed Turn snapshot or accepted stop/close/shutdown receipt is not a task-join promise. Host `stopped` requires the actual descendant and resource-release barriers.


## Canonical controls and receipt queries

`agent_stop`, `session_pause/resume/load/unload/close/cancel` and `turn_cancel`
accept the shared operation request types and return their canonical receipts.
Each direct owner forms the facts under the same short lock that accepts the
mutation. Later load, save, stop or close progress does not rewrite that receipt.
Older convenience calls share these mutations rather than separate business paths.

`receipt_get` reads original-kind creation/control facts from their current owners.
An independent invocation may reuse an operation ID; it never replaces or replays
another invocation. No retained match is `HOST_RECEIPT_NOT_FOUND` (1202), multiple
matches are `HOST_RECEIPT_AMBIGUOUS` (1203), and `receipts` exposes the separate
retained facts. Missing capability remains `HOST_METHOD_UNAVAILABLE` (1401).
Reclamation removes the released owner's receipt data normally; an already-returned
close receipt remains valid without keeping that memory Session alive.

## Save and reload boundaries

Session history is written through real storage at Runtime's complete-response, tool-batch and Turn-ending boundaries. Actual partial output and the real outcome are preserved. Primary execution faults and independent persistence faults remain distinct; a completed execution with a failed save is not reported as durably finalized, rerun or appended a second time.

Explicit Session `resume` captures its own failed original history ranges and metadata candidate. It confirms those originals independently, never reruns a model/tool or reappends confirmed bytes, and cannot unblock queued execution while any captured required save still fails. A partial or changed original remains blocked rather than being repaired. Primary execution outcome remains unchanged.

All disk work runs outside owner/access locks. The backend's separate disk-only mutex protects its journal and original pending range. Reload reopens and validates the actual file only when no pending/uncertain history save owns it. `loaded_records()` is an opening baseline, so same-process write -> unload -> load does not reuse it as a stale live cache. A failed reload does not replace retained memory with an unvalidated prefix, truncate files or skip bad records.

The data-root lease lasts through accepted work, including memory execution and detached caller waits. It does not serialize unrelated Sessions. Controlled Host shutdown retains the directory through genuine joins, pending saves and cleanup, then drops the resource graph even if old observers or Host handles remain. Persistent Sessions are not implicitly closed or deleted.

Closed memory Sessions are removed only after their work and resources retire. Their results and body reads then return missing-source Faults, not retained tombstones. Empty memory Agents retire at the same owner boundary that admits new child creation; persistent Agents remain. An ordinary completed/cancelled Turn or a retained inbox does not by itself reclaim a Session.

The first version has no recovery entry for an unpublished creation whose necessary save failed. Such failures cannot be relabeled successful controlled shutdown, and repeated shutdown is not a retry. Dropping the final root `Arc<Host>` after an already-failed save without controlled shutdown is uncontrolled instance destruction, not a `stopped` claim. No permanent self-retention architecture is provided for that case.

## Verification and remaining scope

Tests use temporary files, production owners, real Runtime and explicitly named scripted/gated model fixtures. The HTTP integration test uses the real provider adapter against a local test server. These are not live-provider or TUI validation.

Run `scripts/check.sh` from the workspace root for formatting, tests, strict
all-target Clippy and rustdoc. Tasks 006, 008, 009 and 012 preserve their original
checkpoints; current composed results and remaining acceptance work are recorded
in `../../PROJECT.md`, not inferred from those older counts.

Current owner modules implement configuration/reload, explicit owned deletion,
canonical controls, delegation/result handoff, output/event reads and running
interaction answers. `host-transport` provides the shared typed/stdio/WSS routes.
These implementation boundaries do not claim actual TUI integration.

## Session-owned compaction

`session_compact` accepts manual utility work directly at an idle usable Session.
It shares admission with Turn preparation, creates no Turn, and leaves both
message queues writable but unconsumed until the work and its save settle.
Explicit pause survives completion. `session_compact_cancel` targets the original
manual operation only; a late cancellation cannot cancel a later compaction.
The original receipt retains its formation-time facts. Session state exposes
current activity; late cancellation reads the original retained outcome.

Automatic compaction runs at real Runtime prompt boundaries with the public
Turn's frozen model and settings. Only unhanded inputs and the final incomplete
accepted exchange are protected; earlier saved batches from that same Turn can
be covered. Utility usage is separate from normal response indexes and usage.

Raw native history remains append-only. The latest confirmed coverage marker
projects one summary plus uncovered raw history for the model, while original
content and TurnResult slicing remain intact. Marker saves use the existing
Session journal boundary. Cancel-before-commit performs no marker append;
commit-before-cancel retains the necessary save and does not claim rollback.
A dropped Runtime prompt waiter cannot discard that work or allow its terminal
history callback to accept a competing batch. An explicit `session_resume`
continues only the original failed save and candidate without rerunning utility
or tools. Force/close and resource retirement retain their real worker joins.

Tasks 037/040 record owner and typed/stdio/WSS coverage; tasks 088/092 cover the
later diagnostic and original-Fault handoff changes. No live-model or TUI
validation is claimed.

## Source-owned observation

`Host::events_subscribe` returns a local `EventSubscription` with its fixed
`SubscribeResult` baseline and a `recv` stream. `host-transport` carries these
same payloads through typed subscriptions and stdio/WSS push notifications.
Registration is connection-bound and precedes per-owner baseline collection.
Host/Agent/Session/Turn scopes continue covering later direct descendants.
Dropping a handle or ending its connection never cancels Session work.

`turn_output_read` and `toolcall_output_read` use explicit UTF-8 `[start, end)`
byte ranges. `end` is fixed for one catch-up operation; `max_bytes` chooses a
page, rounding forward only when needed to return one complete UTF-8 character.
Streamed text, complete Runtime history, execution completion and persistence
confirmation remain distinct facts. Model response indices continue through
internal Loops within one public Turn. Tool identity includes the original Turn.

Trusted resource assemblers can override `ResourceResolver::bind_tool_output`
to wrap already-prepared tools with `ToolOutputBinding::wrap`. This is an
infallible, non-I/O sink binding, not resource selection or business permission
preparation. During the real call, `ToolOutputSink::current()` provides the
bound process-output capability. Clones become inactive when that call returns
or is cancelled, and never hold a Session alive. Final ToolResult text is not
re-emitted as invented process output.

The local Runtime retry-state integration reads `LoopHandle::watch_state`
directly. `TurnInfo` and `turn.state` share actual attempt counts, the original
response index, retry phase, structured Fault and source RFC3339 retry time.
A waiting cancellation does not increment the actual-call count. Retry clearing
is a normal state change, including for state-only subscribers.

`Host::history_read` pages adapted conversation items with an opaque Session/run
cursor. Already-loaded history copies only the requested page. Cold persistent
reads use the real SessionStore read path without creating a Runner, changing
residency or restoring old Turn/ToolCall/output objects. The response does not
expose JSONL rows or internal Loop identities. Public interaction IDs are stable
Host-format IDs mapped by the original Session to actual Runtime interactions;
request, state and settlement projections reuse them. `Host::answer_interaction`
routes through the original Session interaction owner and is exposed by the
shared transport operation table.

`Host::turn_result_snapshot` and `wait_turn_snapshot` also read actual saved
terminal results after persistent reopen, without restoring old execution.
Retained results include their original state/config/LoadId; saved-only results
carry `saved_terminal` and stored content, not invented execution metadata.
Current active/finalizing ownership takes precedence over disk reads. See
`tests/task087_saved_result_restart.rs` and the validated branches in `src/query.rs`.
