# Public method implementation map

Current executable source `4187899ef9b7301ed5af4cf36b84b5825fb83bed` has all 52 real shared routes, including reviewed Agent/Session deletion. Route presence is not full contract acceptance: bound-Turn configuration, public query projections and original terminal revision corrections remain in their independent review stage. The table below is explicitly the preserved older 76a7bfb checkpoint, not the current implementation map. A complete current matrix will accompany the fixed query contracts; see [the executable handoff](executable-review-handoff.md).

## Historical 76a7bfb method map

This checkpoint describes local Host code/test checkpoint `76a7bfbe7f70d7f707d48cc38190829d14c137f2` (published Runtime `69faf9196991524b5549499d6208a1eaf567bf56`, version `0.10.0`), not the full v4.93 release. The 52 names come from `host-protocol::METHOD_CATALOG`; a DTO or catalog entry is not an implemented handler.

`Partial` means a real in-process owner primitive or reusable backend exists, with the stated limits. It does **not** claim a complete canonical typed/RPC method. Thirty-seven actual primitives now share typed/stdio/WSS routing, shown separately below; production process assembly is not yet integrated. Routing alone does not complete a primitive's remaining SPEC contract. `Not implemented` means there is no business implementation to advertise. There are no fake-success handlers for staged methods.

| Canonical method | Core status | Typed/stdio/WSS route | Existing implementation and remaining boundary |
| --- | --- | --- | --- |
| `host.info` | Partial | Routed | `Host::info`. Identity, incarnation, lifecycle and operation-right state; actual shutdown strategy/fault state; logging and complete resource configuration pending. |
| `host.config.get` | Implemented typed query | Routed | `Host::config_get` returns canonical HostConfig and committed revision, without secret values. All three adapters use this same owner query. |
| `host.reload` | Implemented typed entry | Routed | `Host::reload` owns accepted resource preparation, versioned durable publication, immutable original receipts and actual-change-only network key invalidation. All three adapters share the owner path; executable configuration startup remains separate. |
| `host.receipt.get` | Partial | Routed | `Host::receipt_get` traverses direct owners for original-kind creation/control facts; 1202 means missing, 1203 means ambiguous independent invocations. Additional command families remain staged. |
| `host.shutdown` | Implemented typed entry | Routed | `Host::shutdown` uses the canonical request/receipt and owns actual graceful/force cleanup. All three adapters share this real entry; process assembly remains pending. |
| `agent.create` | Partial | Routed | `Host::create_agent_async`. Real memory/persistent creation, owned waits, partial receipts and invocation-isolated tracking; canonical request/receipt mapping pending. |
| `agent.get` | Partial | Routed | `Host::agent_info`. Direct Agent record and committed configuration; complete derived status mapping pending. |
| `agent.list` | Partial | Routed | `Host::agents`. Actual registered owners; load failures are separately observable. |
| `agent.update` | Partial | Not routed | Direct owner APIs update name/Role/default config with versioned necessary saves. The internal agent tool uses these real owners; canonical external command Receipt is pending. |
| `agent.start` | Partial | Not routed | `Host::start_agent`. Owner scheduling gate; complete typed control receipt mapping pending. |
| `agent.stop` | Implemented typed entry | Routed | `Host::agent_stop` forms actual stopping/stopped facts at the Agent boundary and preserves child inboxes; owned stopping continues independently. |
| `agent.set_default_session` | Partial | Not routed | `Host::set_default_session / set_default_session_async`. Memory and confirmed durable default selection; memory children are not restored as durable defaults. Canonical receipt mapping pending. |
| `agent.delete` | Not implemented | Not routed | Required by SPEC; no completed business handler. |
| `session.create` | Partial | Routed | `Host::create_session_async`. Real owner-scoped creation and explicit modes; published objects survive later delivery failure. |
| `session.get` | Partial | Routed | `Host::session_info`. Committed effective config and direct ownership; complete canonical record mapping pending. |
| `session.list` | Partial | Routed | `Host::sessions`. Direct-child records; unhealthy persisted children have explicit load-failure facts. |
| `session.update` | Partial | Not routed | Direct owner APIs update title/effective config with durable confirmation and frozen current Turn. Successful updates clear only the three recoverable resource/configuration startup faults; canonical external Receipt remains work. |
| `session.state` | Partial | Routed | `Host::session_state`. Actual queues, active Turn, residency, saving and faults; final canonical state schema mapping pending. |
| `session.load` | Implemented typed entry | Routed | `Host::session_load` returns actual load/residency and pause facts; owned validation continues without unpausing. |
| `session.unload` | Implemented typed entry | Routed | `Host::session_unload` preserves needed memory context and original failed ranges; queued inputs cause an explicit unload pause reason. |
| `session.pause` | Implemented typed entry | Routed | `Host::session_pause` forms paused facts at acceptance without cancelling the current Turn. |
| `session.resume` | Implemented typed entry | Routed | Real load/save/pause ownership and original-save continuation. Explicit resource repair permits one fresh preparation of original queued input; repeated failure leaves its actual Fault, and history/save blocks remain intact. |
| `session.cancel` | Implemented typed entry | Routed | `Host::session_cancel` reports the actual selected current Turn or no active Turn; only the first cancellation claim clears its accepted inbox range. |
| `session.close` | Implemented typed entry | Routed | Real closing/closed facts, accepted cleanup and source reclamation, including registered delegation-result transfer before temporary owner release. |
| `session.delete` | Not implemented | Not routed | Required by SPEC; no completed business handler. |
| `message.send` | Partial | Routed | Real Session acceptance and dual queues. Trusted internal agent provenance uses the same receive boundary; external JSON cannot claim it. |
| `message.get` | Partial | Routed | Retained source, in_reply_to, queued/bound/removed facts, without guessing missing execution. |
| `input.list` | Partial | Not routed | `Session snapshot queues`. Both independent FIFO queues are observable; dedicated canonical query mapping pending. |
| `input.remove` | Partial | Not routed | `Host::remove_input`. Only an unbound queued input may be removed. |
| `turn.get` | Partial | Routed | `Host::turn_info`. Real active/completed execution metadata with independent outcome and persistence. |
| `turn.list` | Partial | Routed | `Host::turns`. Retained Session-owned Turn records in completion order. |
| `turn.wait` | Partial | Routed | `Host::wait_turn`. Waits the selected owner result; typed/stdio wait budget and disconnect ownership are tested; complete result schema remains staged. |
| `turn.cancel` | Implemented typed entry | Routed | `Host::turn_cancel` retains the exact Turn cancellation claim/state/revision; stale or repeated calls never clear later inputs or retarget another Turn. |
| `turn.result` | Partial | Routed | `Host::turn_result`. Actual native history/partial and primary/persistence faults while retained; complete durable query reconstruction pending. |
| `turn.output.read` | Implemented typed query | Routed | Actual owner-retained per-response output with UTF-8 byte ranges and fixed endpoints; missing/reclaimed source is a Fault. |
| `model.list` | Implemented typed query | Routed | Current configured complete model references and explicit capability descriptions; shared typed/stdio/WSS query. |
| `tool.list` | Implemented typed query | Routed | Actual static ToolSpecs or direct Agent/Session selection; missing configured resources produce Fault. Shared typed/stdio/WSS query. |
| `session.history.read` | Implemented typed query | Routed | Actual complete/partial Host history with provenance and fixed pagination; cold JSONL reads create no old Turn or Runtime and do not rewrite storage. |
| `session.context` | Not implemented | Not routed | Required by SPEC; no completed business handler. |
| `session.compact` | Implemented typed entry | Routed | `Host::session_compact` owns manual utility work, frozen resources, append-only coverage and necessary save barriers. Automatic request-boundary compaction uses the same owner; original receipt facts do not mutate. |
| `session.compact.cancel` | Implemented typed entry | Routed | `Host::session_compact_cancel` targets the exact operation and reports only actual cancellation claims. Late/stale requests cannot cancel a later operation or skip required save settlement. |
| `workspace.files` | Partial | Not routed | `host-workspace::Workspace::files` returns actual scoped file entries; Session authorization/Fault routing and transport are pending. |
| `workspace.read` | Partial | Not routed | `Workspace::read` preserves requested UTF-8 byte ranges; Session authorization/Fault routing and transport are pending. |
| `workspace.search` | Partial | Not routed | `Workspace::search` performs actual scoped literal search with explicit continuation; Session routing is pending. |
| `workspace.status` | Partial | Not routed | `Workspace::status` reads actual Git state without invented clean results; Session routing is pending. |
| `changes.list` | Partial | Not routed | `Workspace::changes_list` reads unattributed Git changes; `EditJournal::list` reads explicitly captured Session/Turn observations. Real write-tool capture and Session retention/persistence remain pending. |
| `changes.diff` | Partial | Not routed | `Workspace::changes_diff` and `diff_edit` compare actual Git/file or captured before/after bytes. Canonical routing and write-tool recording remain pending. |
| `toolcall.get` | Implemented typed query | Routed | Actual invocation input, status and source association from owner facts, never reconstructed from presentation text. |
| `toolcall.output.read` | Implemented typed query | Routed | Actual per-call retained process/result output and fixed UTF-8 ranges, respecting source lifetime. |
| `interaction.answer` | Implemented typed entry | Routed | Original Runtime waiting point validates identity, Load, type and one settlement; answered/cancelled/expired facts remain distinct. Independent deadline correction review passed. |
| `events.subscribe` | Implemented typed entry | Routed | Connection-owned scope/category subscriptions, dynamic direct descendants, initial state and exact source watermarks, early push/resync and real stdio/WSS delivery. |
| `events.unsubscribe` | Implemented typed entry | Routed | Exact subscription cleanup without dropping another stream, the shared connection or accepted execution; stale ends do not target replacements. |

## Existing evidence

- `crates/host-protocol/tests/`: strict contracts, faults, receipt combinations and exactly 52 catalog methods. These tests do not execute business methods.
- `crates/host-core/tests/runtime_owners.rs`: real Host/Agent/Session/Runtime acceptance, independent queues, both finish/steer orderings, config binding, cancellation, operation rights, failure pause and owner settlement. Most run on multiple Tokio worker threads.
- `crates/host-core/src/session/tests.rs`: real temporary JSONL and Runtime save boundaries, cancellation during saving, authoritative partial and simultaneous execution/save failure.
- `crates/host-core/tests/persistent_owners.rs`: real persistent startup, identity/restart, corruption isolation, owned asynchronous creation, explicit storage modes, partial creation failures, confirmed metadata updates, close/default ordering, stop wakeups and root-lease lifetime.
- `crates/host-core/tests/host_shutdown.rs`, `entity_retirement.rs`, and `history_continuation.rs`: actual shutdown/GC, original save continuation, model/tool cancellation and root-release barriers.
- `crates/host-core/tests/http_host.rs`: two turns through real owners/Runtime and the production provider over loopback TCP/HTTP/SSE. No live external model or TUI is implied.
- `crates/host-storage/tests/` and storage unit tests: actual filesystem locking, identity checks, atomic object saves, strict JSONL, durability/reconciliation boundaries and failure injection.
- `crates/host-workspace/tests/`: 30 actual temporary-file/Git/edit-snapshot tests; no Session/transport/write-tool integration claim.
- `crates/host-provider/tests/http_runtime.rs`: production adapter with real HTTP/SSE fixtures and actual Runtime; complete, partial, error, retry and tool-history paths.

## Cross-cutting open scope

Remaining canonical metadata/input/start/default commands, production-process assembly, production local-tool/workspace/write capture, context, persistent deletion and business diagnostics remain separate work. Reload and compaction controls are now routed; the standalone ordinary-tool and diagnostics modules are included. TUI integration is deferred. Resource fault recovery is independently verified. Current output/subscriptions/DSH/interaction implementations are real but still require full product acceptance. Internal targeted review is useful evidence but does not replace the requested external Host review.

The targeted review findings for memory-owner root leases, empty-queue resume validation, unsafe-child isolation, duplicate caller operation IDs, worker retirement, late stop wakeups and initialization fencing are fixed in the recorded commits. `a088b00` adds file-scoped metadata snapshot/publication coordination; `d73b161` limits close dependencies to actual current or pending default references. The complete workspace gate at `0b9b390` passes 198 unit/integration tests and 2 doctests against the published Runtime revision. The same gate passed from a fresh local Git clone with an empty per-checkout build target and no local patch. Targeted follow-up review found no new blocker in these fixes; its seven additional regressions are now permanent tests. This is not full lifecycle or release completion, and the requested external review is still pending.

The integrated lifecycle checkpoint `413916c` passes the complete 240-test/2-doctest gate (core 100, protocol 47, provider 36, storage 57). A scoped independent review passed 12 additional public-API cases; the normal concurrent-close handoff it found was fixed before integration. There is no new unpublished-creation recovery entry or implicit repeated-shutdown retry. Dropping the final root Host after an already-failed save without controlled shutdown is an explicit uncontrolled-destruction limit, not a successful stopped state. Canonical controls were then integrated through `b961f9f`; their complete gate passes 254 tests and 2 doctests. Independent review identified the lookup error conflation corrected in that commit; fixed correction review passed 161 core/protocol cases with no remaining scoped finding. Typed/stdio now routes 25 real primitives; that availability is distinct from complete canonical contract coverage, as the separate column records.

Workspace backend integration `b0fdbfb` passes the complete 275-test/2-doctest gate. Unique cases were verified using `--list`, excluding storage subprocess output duplication. Workspace scoped review and its fixes are recorded below; the six canonical methods are not available merely because their backend exists.

At fixed typed/stdio checkpoint `8079970`, the entire gate passes 290 unique
unit/integration tests and 3 doctests. The count is verified by `--list`. Scoped
adapter review found the two client corrections recorded below. Workspace review identified real external-filter
execution and damaged-repository classification defects; the fixes are integrated at `775235a` and independently verified under stable
Git configuration/attributes. Uncoordinated concurrent changes remain a declared
first-version limit. The integrated full gate passes 299 unique tests and 3
doctests, not complete canonical method acceptance.

WSS checkpoint `bd8bf2f` reuses those same 25 routes through actual TLS and
authenticated initialization, with network-only key-change invalidation. The
complete gate passes 321 unique tests and 3 doctests. This is an embedding API,
not complete process/reload assembly. Shared-client unknown-Fault and synchronous
preparation corrections are integrated through `5921c72`. Its full gate and
independent scoped recheck pass 331 unique tests and 3 doctests; the original
five probes, four additional actual WSS probes and repeated focused cases also
pass. This does not complete product-wide method acceptance.

Runtime 0.10 migration and the actual published dependency pin at `2751106` add
nine real HTTP/Host regressions, bringing the integrated gate to 340 unique
tests and 3 doctests without a local dependency override. The 25-route count
is unchanged; ongoing observation/DSH topic work is not yet included here.

## Later owner/resource integration

Local `04ba5bc` passes 494 unique tests and 3 doctests against actual published Runtime `69faf919`. Its 34 routes include standard interactions, observation and resource queries; prepared reload publication is implemented but the canonical reload command/receipt is still absent. Task 029 distinguishes integrated scope, independent findings and current unfinished paths. Tasks 018–025 retain their own source checkpoints and validation boundaries; their historical local-patch runs are not confused with this actual-Git gate.

## Reviewed execution integration

Fixed local code `76a7bfbe7f70d7f707d48cc38190829d14c137f2` passes **650 unique tests and 4 doctests**. Canonical reload and two compaction controls raise the shared routing count from 34 to 37. Tasks 030/035 cover reload, 031/036/037/040 compaction, 032/038 local-tool modules and 039 diagnostics. [The fixed handoff](execution-review-handoff.md) distinguishes this integrated tree from later production tool/application/workspace/control/context topics and preserves previous checkpoints.
