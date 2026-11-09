# In-process owners

`minicore-host-core` composes the real Host -> Agent -> SessionController ownership path with Runtime. Memory and persistent Sessions use the same queues, configuration binding, Runtime loop and result handling. A connection grants access; it never replaces the target's direct owner.

This crate is an in-process foundation, not the complete v4.93 canonical service or a transport server. Runtime integration requires the extended Runtime revision pinned by the workspace. Local development patches are not a clean-clone reproducibility guarantee; repository provenance and integration records identify the exact validated source.

## Persistent initialization

`Host::open_persistent(path, initial_settings, resources).await` acquires the exclusive data root before constructing executable owners. A new root saves its identity; an existing root uses its saved Host ID and supported settings. Each open receives a fresh incarnation. `HostSettings` currently contains only identity/display metadata (`name`), not provider, tool, access, logging or network configuration. The embedding application still supplies `ResourceResolver` explicitly.

The Host reads direct Agents. Each Agent reads its own metadata and initializes its direct Session metadata independently. History remains unloaded until the Session's load/resume/execution path validates it. Stored execution handles, queues, pauses, Load IDs, active Turns and tool processes are never reconstructed. Closed Sessions remain closed.

Malformed or unsafe child entries remain associated with their actual IDs through `agent_load_failures` or `session_load_failures`. They do not silently become empty objects or hide healthy siblings. An unavailable default remains unavailable; it is not redirected or replaced automatically. Unsupported records/settings are rejected without overwriting saved values with defaults.

P/P, P/M and M/M are supported. M/P is rejected. Memory descendants of a persistent Host retain its root lease during accepted work but create no identity/history files of their own.

## Acceptance and completion

- `create_agent_async` and `create_session_async` register owned work before disk I/O. Aborting the caller's wait or revoking its connection does not cancel that work. The returned receipt describes actual completed creation steps, partial failure, and initial-message delivery facts. An accepted initial message may still have an independently running Turn.
- Every creation invocation has independent work state. `operation_id` is correlation data, not an idempotency or replay key. `creation_receipts` and `session_creation_receipts` return all matching invocations; the singular queries report ambiguity when a caller reuses an ID. Published object/message IDs are never borrowed from another invocation.
- `update_agent_async`, `update_session_async` and `set_default_session_async` await confirmed metadata saves before publishing effective values, versions or defaults. Queryable `metadata_saving`/`metadata_fault` fields expose unfinished or failed work. Agent metadata changes are accepted in Active/Stopped, not while initialization or stop is still running.
- `reconcile_agent_record` and `reconcile_session_record` explicitly continue the original failed metadata candidate. They check the actual record against the original baseline or exact candidate. A different file is not overwritten, and another candidate cannot replace the retained pending save. These APIs do not reconcile Runtime history saves.
- `load_session` accepts owned loading work. `resume` validates required unloaded history even with an empty inbox and clears allowed pause reasons only after successful loading. A later explicit pause wins over an earlier pending resume. Inspect the Session snapshot/subscription for loading completion or its real blocking fault.
- `close_session` fences reception and starts owned close work. The direct Agent confirms clearing its durable default before the Session saves and publishes `closed`. Failure leaves a visible `closing` state and the original save context. A memory default is represented as null on disk and is never recreated on restart.
- `stop_agent` fences new execution/creation and waits for its accepted work and children. Child state changes wake the parent's aggregate without cross-owner state access. `start_agent` resumes Stopped or is a no-op for Active; it cannot promote unfinished Creating/Stopping/retirement states.

The synchronous creation/configuration convenience entries support memory ownership. Persistent mutation uses the asynchronous entries; it never silently substitutes a memory backend. A completed creation or metadata waiter follows retirement of that temporary worker. A completed Turn snapshot or accepted stop/close response is not a complete Host shutdown or a task-join promise.

## Save and reload boundaries

Session history is written through real storage at Runtime's complete-response, tool-batch and Turn-ending boundaries. Actual partial output and the real outcome are preserved. Primary execution faults and independent persistence faults remain distinct; a completed execution with a failed save is not reported as durably finalized, rerun or appended a second time.

All disk work runs outside owner/access locks. The backend's separate disk-only mutex protects its journal and original pending range. Reload reopens and validates the actual file only when no pending/uncertain history save owns it. `loaded_records()` is an opening baseline, so same-process write -> unload -> load does not reuse it as a stale live cache. A failed reload does not replace retained memory with an unvalidated prefix, truncate files or skip bad records.

The data-root lease lasts through accepted work, including memory execution and detached caller waits. It does not serialize unrelated Sessions. Controlled Host shutdown is not yet provided by this crate; simply dropping the public Host handle is not a substitute for a future graceful/force shutdown contract.

## Verification and remaining scope

Tests use temporary files, production owners, real Runtime and explicitly named scripted/gated model fixtures. The HTTP integration test uses the real provider adapter against a local test server. These are not live-provider or TUI validation.

Run formatting, workspace tests, all-target Clippy with warnings denied, and rustdoc with warnings denied in the configured development checkout. Detailed checkpoints, commands and known review findings are in `tasks/006-persistent-owners/TASK.md` at the repository root.

Not implemented here: complete Host execution-resource configuration/reload and controlled shutdown, automatic memory-entity reclamation, phased directory deletion, owner-level continuation of failed Runtime history saves, the complete canonical operation surface, transport/SDK adapters, DSH, interaction and compaction. API/type presence is not evidence that these features are complete.
