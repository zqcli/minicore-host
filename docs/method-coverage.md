# Public method implementation map

This checkpoint describes Host `470f136` (core creation checkpoint `c5e6aca`), not the full v4.93 release. The 52 names come from `host-protocol::METHOD_CATALOG`; a DTO or catalog entry is not an implemented handler.

`Partial` means a real in-process owner primitive exists, with the stated limits. It does **not** claim a complete canonical typed/RPC method. No stdio/WSS dispatcher is present yet. `Not implemented` means there is no business implementation to advertise. There are no fake-success handlers for staged methods.

| Canonical method | Status | Existing implementation and remaining boundary |
| --- | --- | --- |
| `host.info` | Partial | `Host::info`. Identity, incarnation, lifecycle and operation-right state; logging/config/complete shutdown state pending. |
| `host.config.get` | Partial | `Host::settings`. Only supported Host name metadata; not execution, access or logging configuration. |
| `host.reload` | Not implemented | Required by SPEC; no completed business handler. |
| `host.receipt.get` | Partial | `Host::receipt / session_creation_receipt`. Retained real creation progress; canonical common query mapping pending. |
| `host.shutdown` | Not implemented | Required by SPEC; no completed business handler. |
| `agent.create` | Partial | `Host::create_agent_async`. Real memory/persistent creation, owned waits and partial receipts; lifecycle review fixes pending. |
| `agent.get` | Partial | `Host::agent_info`. Direct Agent record and committed configuration; complete derived status mapping pending. |
| `agent.list` | Partial | `Host::agents`. Actual registered owners; load failures are separately observable. |
| `agent.update` | Partial | `Host::update_agent`. Memory configuration update; persistent save/commit path in progress. |
| `agent.start` | Partial | `Host::start_agent`. Owner scheduling gate; complete typed control receipt mapping pending. |
| `agent.stop` | Partial | `Host::stop_agent`. Cancels active children with inbox preserved; stop owner is tracked; canonical receipt mapping pending. |
| `agent.set_default_session` | Partial | `Host::set_default_session`. Memory pointer selection; persistent publication ordering in progress. |
| `agent.delete` | Not implemented | Required by SPEC; no completed business handler. |
| `session.create` | Partial | `Host::create_session_async`. Real owner-scoped creation and explicit modes; published objects survive later delivery failure. |
| `session.get` | Partial | `Host::session_info`. Committed effective config and direct ownership; complete canonical record mapping pending. |
| `session.list` | Partial | `Host::sessions`. Direct-child records; unhealthy persisted children have explicit load-failure facts. |
| `session.update` | Partial | `Host::update_session`. Memory versioned config; same-Turn binding remains fixed. Persistent update in progress. |
| `session.state` | Partial | `Host::session_state`. Actual queues, active Turn, residency, saving and faults; final canonical state schema mapping pending. |
| `session.load` | Partial | `Persistent initialization / lazy start`. History is loaded and validated before execution; independent public load owner in progress. |
| `session.unload` | Partial | `Host::unload`. Memory context retained; complete persistent unload/load control in progress. |
| `session.pause` | Partial | `Host::pause`. Explicit pause takes effect without cancelling current Turn; canonical receipt mapping pending. |
| `session.resume` | Partial | `Host::resume`. Memory scheduling works; persistent empty-queue validation review fix in progress. |
| `session.cancel` | Partial | `Host::cancel_session`. Owner current-Turn cancellation; first claim owns optional inbox clear. |
| `session.close` | Partial | `Host::close_session`. Memory closing/default clearing; persistent metadata/default ordering in progress. |
| `session.delete` | Not implemented | Required by SPEC; no completed business handler. |
| `message.send` | Partial | `Host::send`. Real Session acceptance, actual queue and typed receipt; internal delegation source path not implemented. |
| `message.get` | Partial | `Host::message`. Retained queued/bound/removed facts; does not infer execution from absence. |
| `input.list` | Partial | `Session snapshot queues`. Both independent FIFO queues are observable; dedicated canonical query mapping pending. |
| `input.remove` | Partial | `Host::remove_input`. Only an unbound queued input may be removed. |
| `turn.get` | Partial | `Host::turn_info`. Real active/completed execution metadata with independent outcome and persistence. |
| `turn.list` | Partial | `Host::turns`. Retained Session-owned Turn records in completion order. |
| `turn.wait` | Partial | `Host::wait_turn`. Waits the selected owner result; shared client timeout/transport behavior not implemented. |
| `turn.cancel` | Partial | `Host::cancel_turn`. Exact Turn claim, stale/repeated cancellation cannot clear later inputs; canonical receipt mapping pending. |
| `turn.result` | Partial | `Host::turn_result`. Actual native history/partial and primary/persistence faults while retained; complete durable query reconstruction pending. |
| `turn.output.read` | Not implemented | Required by SPEC; no completed business handler. |
| `model.list` | Not implemented | Required by SPEC; no completed business handler. |
| `tool.list` | Not implemented | Required by SPEC; no completed business handler. |
| `session.history.read` | Partial | `Host::history / SessionJournal`. Actual loaded native context; public paging/provenance/full persisted-history query mapping pending. |
| `session.context` | Not implemented | Required by SPEC; no completed business handler. |
| `session.compact` | Not implemented | Required by SPEC; no completed business handler. |
| `session.compact.cancel` | Not implemented | Required by SPEC; no completed business handler. |
| `workspace.files` | Not implemented | Required by SPEC; no completed business handler. |
| `workspace.read` | Not implemented | Required by SPEC; no completed business handler. |
| `workspace.search` | Not implemented | Required by SPEC; no completed business handler. |
| `workspace.status` | Not implemented | Required by SPEC; no completed business handler. |
| `changes.list` | Not implemented | Required by SPEC; no completed business handler. |
| `changes.diff` | Not implemented | Required by SPEC; no completed business handler. |
| `toolcall.get` | Not implemented | Required by SPEC; no completed business handler. |
| `toolcall.output.read` | Not implemented | Required by SPEC; no completed business handler. |
| `interaction.answer` | Not implemented | Required by SPEC; no completed business handler. |
| `events.subscribe` | Partial | `Host::session_states`. Revocable connection-scoped Session state watch only; not the canonical scoped/filterable event stream. |
| `events.unsubscribe` | Not implemented | Required by SPEC; no completed business handler. |

## Existing evidence

- `crates/host-protocol/tests/`: strict contracts, faults, receipt combinations and exactly 52 catalog methods. These tests do not execute business methods.
- `crates/host-core/tests/runtime_owners.rs`: real Host/Agent/Session/Runtime acceptance, independent queues, both finish/steer orderings, config binding, cancellation, operation rights, failure pause and owner settlement. Most run on multiple Tokio worker threads.
- `crates/host-core/src/session/tests.rs`: real temporary JSONL and Runtime save boundaries, cancellation during saving, authoritative partial and simultaneous execution/save failure.
- `crates/host-core/tests/persistent_owners.rs`: real persistent startup, identity/restart, corruption isolation, owned asynchronous creation, explicit storage modes and partial creation failures.
- `crates/host-core/tests/http_host.rs`: two turns through real owners/Runtime and the production provider over loopback TCP/HTTP/SSE. No live external model or TUI is implied.
- `crates/host-storage/tests/` and storage unit tests: actual filesystem locking, identity checks, atomic object saves, strict JSONL, durability/reconciliation boundaries and failure injection.
- `crates/host-provider/tests/http_runtime.rs`: production adapter with real HTTP/SSE fixtures and actual Runtime; complete, partial, error, retry and tool-history paths.

## Cross-cutting open scope

Canonical receipt/state mapping, request dispatch, client wait timeout ownership, Host shutdown/reload, network authentication and key changes, output byte ranges, full scoped subscriptions, DSH delegation, built-in tools/workspace/diffs, interactions, compaction, diagnostics and final TUI integration remain separate work. Internal targeted review is useful evidence but does not replace the requested external Host review.

At this checkpoint, the persistent review's memory-owner root lease and empty-queue resume findings are assigned for correction. The unsafe-child enumeration finding is fixed in `470f136`; unchanged actual child reads/writes still enforce safety checks. Do not advertise the persistent lifecycle as complete while these or subsequent findings remain open.
