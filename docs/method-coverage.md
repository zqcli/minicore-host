# Public method implementation map

This checkpoint describes Host code/test checkpoint `b0fdbfb` (published Runtime `18a6ac9b82d003330cd870275efca793cdcc9a96`), not the full v4.93 release. The 52 names come from `host-protocol::METHOD_CATALOG`; a DTO or catalog entry is not an implemented handler.

`Partial` means a real in-process owner primitive or reusable backend exists, with the stated limits. It does **not** claim a complete canonical typed/RPC method. No stdio/WSS dispatcher is present yet. `Not implemented` means there is no business implementation to advertise. There are no fake-success handlers for staged methods.

| Canonical method | Status | Existing implementation and remaining boundary |
| --- | --- | --- |
| `host.info` | Partial | `Host::info`. Identity, incarnation, lifecycle and operation-right state; actual shutdown strategy/fault state; logging and complete resource configuration pending. |
| `host.config.get` | Partial | `Host::settings`. Only supported Host name metadata; not execution, access or logging configuration. |
| `host.reload` | Not implemented | Required by SPEC; no completed business handler. |
| `host.receipt.get` | Partial | `Host::receipt_get` traverses direct owners for original-kind creation/control facts; 1202 means missing, 1203 means ambiguous independent invocations. Additional command families remain staged. |
| `host.shutdown` | Implemented typed entry | `Host::shutdown` uses the canonical request/receipt and owns actual graceful/force cleanup. Transport dispatch remains pending. |
| `agent.create` | Partial | `Host::create_agent_async`. Real memory/persistent creation, owned waits, partial receipts and invocation-isolated tracking; canonical request/receipt mapping pending. |
| `agent.get` | Partial | `Host::agent_info`. Direct Agent record and committed configuration; complete derived status mapping pending. |
| `agent.list` | Partial | `Host::agents`. Actual registered owners; load failures are separately observable. |
| `agent.update` | Partial | `Host::update_agent / update_agent_async`. Memory and durable configuration updates; confirmed publication and retained original-candidate reconciliation work. Canonical receipt mapping pending. |
| `agent.start` | Partial | `Host::start_agent`. Owner scheduling gate; complete typed control receipt mapping pending. |
| `agent.stop` | Implemented typed entry | `Host::agent_stop` forms actual stopping/stopped facts at the Agent boundary and preserves child inboxes; owned stopping continues independently. |
| `agent.set_default_session` | Partial | `Host::set_default_session / set_default_session_async`. Memory and confirmed durable default selection; memory children are not restored as durable defaults. Canonical receipt mapping pending. |
| `agent.delete` | Not implemented | Required by SPEC; no completed business handler. |
| `session.create` | Partial | `Host::create_session_async`. Real owner-scoped creation and explicit modes; published objects survive later delivery failure. |
| `session.get` | Partial | `Host::session_info`. Committed effective config and direct ownership; complete canonical record mapping pending. |
| `session.list` | Partial | `Host::sessions`. Direct-child records; unhealthy persisted children have explicit load-failure facts. |
| `session.update` | Partial | `Host::update_session / update_session_async`. Memory and durable versioned config; effective values publish after confirmation, with fixed same-Turn bindings. Canonical receipt mapping pending. |
| `session.state` | Partial | `Host::session_state`. Actual queues, active Turn, residency, saving and faults; final canonical state schema mapping pending. |
| `session.load` | Implemented typed entry | `Host::session_load` returns actual load/residency and pause facts; owned validation continues without unpausing. |
| `session.unload` | Implemented typed entry | `Host::session_unload` preserves needed memory context and original failed ranges; queued inputs cause an explicit unload pause reason. |
| `session.pause` | Implemented typed entry | `Host::session_pause` forms paused facts at acceptance without cancelling the current Turn. |
| `session.resume` | Implemented typed entry | `Host::session_resume` forms actual loading/save/pause facts, preserves a later pause, and confirms only its own original saves before execution continues. |
| `session.cancel` | Implemented typed entry | `Host::session_cancel` reports the actual selected current Turn or no active Turn; only the first cancellation claim clears its accepted inbox range. |
| `session.close` | Implemented typed entry | `Host::session_close` returns its formed closing/closed receipt even if memory reclamation follows immediately; default and save ordering remain owned. Delegation scope integration remains open. |
| `session.delete` | Not implemented | Required by SPEC; no completed business handler. |
| `message.send` | Partial | `Host::send`. Real Session acceptance, actual queue and typed receipt; internal delegation source path not implemented. |
| `message.get` | Partial | `Host::message`. Retained queued/bound/removed facts; does not infer execution from absence. |
| `input.list` | Partial | `Session snapshot queues`. Both independent FIFO queues are observable; dedicated canonical query mapping pending. |
| `input.remove` | Partial | `Host::remove_input`. Only an unbound queued input may be removed. |
| `turn.get` | Partial | `Host::turn_info`. Real active/completed execution metadata with independent outcome and persistence. |
| `turn.list` | Partial | `Host::turns`. Retained Session-owned Turn records in completion order. |
| `turn.wait` | Partial | `Host::wait_turn`. Waits the selected owner result; shared client timeout/transport behavior not implemented. |
| `turn.cancel` | Implemented typed entry | `Host::turn_cancel` retains the exact Turn cancellation claim/state/revision; stale or repeated calls never clear later inputs or retarget another Turn. |
| `turn.result` | Partial | `Host::turn_result`. Actual native history/partial and primary/persistence faults while retained; complete durable query reconstruction pending. |
| `turn.output.read` | Not implemented | Required by SPEC; no completed business handler. |
| `model.list` | Not implemented | Required by SPEC; no completed business handler. |
| `tool.list` | Not implemented | Required by SPEC; no completed business handler. |
| `session.history.read` | Partial | `Host::history / SessionJournal`. Actual loaded native context; public paging/provenance/full persisted-history query mapping pending. |
| `session.context` | Not implemented | Required by SPEC; no completed business handler. |
| `session.compact` | Not implemented | Required by SPEC; no completed business handler. |
| `session.compact.cancel` | Not implemented | Required by SPEC; no completed business handler. |
| `workspace.files` | Partial | `host-workspace::Workspace::files` returns actual scoped file entries; Session authorization/Fault routing and transport are pending. |
| `workspace.read` | Partial | `Workspace::read` preserves requested UTF-8 byte ranges; Session authorization/Fault routing and transport are pending. |
| `workspace.search` | Partial | `Workspace::search` performs actual scoped literal search with explicit continuation; Session routing is pending. |
| `workspace.status` | Partial | `Workspace::status` reads actual Git state without invented clean results; Session routing is pending. |
| `changes.list` | Partial | `Workspace::changes_list` reads unattributed Git changes; `EditJournal::list` reads explicitly captured Session/Turn observations. Real write-tool capture and Session retention/persistence remain pending. |
| `changes.diff` | Partial | `Workspace::changes_diff` and `diff_edit` compare actual Git/file or captured before/after bytes. Canonical routing and write-tool recording remain pending. |
| `toolcall.get` | Not implemented | Required by SPEC; no completed business handler. |
| `toolcall.output.read` | Not implemented | Required by SPEC; no completed business handler. |
| `interaction.answer` | Not implemented | Required by SPEC; no completed business handler. |
| `events.subscribe` | Partial | `Host::session_states`. Revocable connection-scoped Session state watch only; not the canonical scoped/filterable event stream. |
| `events.unsubscribe` | Not implemented | Required by SPEC; no completed business handler. |

## Existing evidence

- `crates/host-protocol/tests/`: strict contracts, faults, receipt combinations and exactly 52 catalog methods. These tests do not execute business methods.
- `crates/host-core/tests/runtime_owners.rs`: real Host/Agent/Session/Runtime acceptance, independent queues, both finish/steer orderings, config binding, cancellation, operation rights, failure pause and owner settlement. Most run on multiple Tokio worker threads.
- `crates/host-core/src/session/tests.rs`: real temporary JSONL and Runtime save boundaries, cancellation during saving, authoritative partial and simultaneous execution/save failure.
- `crates/host-core/tests/persistent_owners.rs`: real persistent startup, identity/restart, corruption isolation, owned asynchronous creation, explicit storage modes, partial creation failures, confirmed metadata updates, close/default ordering, stop wakeups and root-lease lifetime.
- `crates/host-core/tests/host_shutdown.rs`, `entity_retirement.rs`, and `history_continuation.rs`: actual shutdown/GC, original save continuation, model/tool cancellation and root-release barriers.
- `crates/host-core/tests/http_host.rs`: two turns through real owners/Runtime and the production provider over loopback TCP/HTTP/SSE. No live external model or TUI is implied.
- `crates/host-storage/tests/` and storage unit tests: actual filesystem locking, identity checks, atomic object saves, strict JSONL, durability/reconciliation boundaries and failure injection.
- `crates/host-workspace/tests/`: 21 actual temporary-file/Git/edit-snapshot tests; no Session/transport/write-tool integration claim.
- `crates/host-provider/tests/http_runtime.rs`: production adapter with real HTTP/SSE fixtures and actual Runtime; complete, partial, error, retry and tool-history paths.

## Cross-cutting open scope

Remaining canonical command/state mapping, request dispatch, client wait timeout ownership, Host execution configuration/reload, network authentication and key changes, output byte ranges, full scoped subscriptions, DSH delegation, built-in tools/workspace/diffs, interactions, compaction, diagnostics and final TUI integration remain separate work. Internal targeted review is useful evidence but does not replace the requested external Host review.

The targeted review findings for memory-owner root leases, empty-queue resume validation, unsafe-child isolation, duplicate caller operation IDs, worker retirement, late stop wakeups and initialization fencing are fixed in the recorded commits. `a088b00` adds file-scoped metadata snapshot/publication coordination; `d73b161` limits close dependencies to actual current or pending default references. The complete workspace gate at `0b9b390` passes 198 unit/integration tests and 2 doctests against the published Runtime revision. The same gate passed from a fresh local Git clone with an empty per-checkout build target and no local patch. Targeted follow-up review found no new blocker in these fixes; its seven additional regressions are now permanent tests. This is not full lifecycle or release completion, and the requested external review is still pending.

The integrated lifecycle checkpoint `413916c` passes the complete 240-test/2-doctest gate (core 100, protocol 47, provider 36, storage 57). A scoped independent review passed 12 additional public-API cases; the normal concurrent-close handoff it found was fixed before integration. There is no new unpublished-creation recovery entry or implicit repeated-shutdown retry. Dropping the final root Host after an already-failed save without controlled shutdown is an explicit uncontrolled-destruction limit, not a successful stopped state. Canonical controls were then integrated through `b961f9f`; their complete gate passes 254 tests and 2 doctests. Independent review identified the lookup error conflation corrected in that commit; fixed correction review passed 161 core/protocol cases with no remaining scoped finding. Typed/stdio adapters and workspace reads remain active work, not completed methods in this map.

Workspace backend integration `b0fdbfb` passes the complete 275-test/2-doctest gate. Unique cases were verified using `--list`, excluding storage subprocess output duplication. Workspace independent review is pending; the six canonical methods are not available merely because their backend exists.
