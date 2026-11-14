# Host tools

The statically compiled `agent` tool reuses Runtime `ToolSpec`, `Tool`, and
`ToolSet`. Its model inputs expose only the six actions in SPEC v4.11.0 §7.
Identity, operation/message ID generation, source capabilities, and configuration
revision selection are backend responsibilities, never model-supplied authority.

`TurnPort` is a narrow, Turn-bound owner integration boundary. Implementations
must use a trusted internal capability minted by Session before input claiming,
not a copied client `CallerContext`. Delegation permits and result sinks have
independent lifetimes, matching the registered scope and direct parent. The
adapter must receive necessary actual results before a memory child releases.

## Owner integration

`CoreTurnPort::prepared` consumes the core-minted `PreparedTurnBinding` during
`ResourceResolver::resolve_for_turn`. Pass that port to `ToolRegistry::bind`
with the selected `SessionConfig.tools`, then use the resulting real `ToolSet`
in the returned `ExecutionConfig`. Binding is fallible before Session claims
input. The capability remains inactive until the owner's matching claim and
expires when that public Turn ends. Every step keeps that frozen set.
The later `bind_tool_output` hook is only for infallible output wrapping.

The production bridge forwards operations to core capabilities. It never
creates a connection or CallerContext. Metadata and config candidates come
from one owner snapshot with its original revision. Inspect uses authoritative
Agent/Session state, paged history, model/tool text ranges, messages and
receipts. Output projections exclude Runtime loop and provider replay metadata.
`history_page`, `output_range`, and model `response_index` use the shared read
contracts. A Turn output read without a range shares the public asynchronous
result reader, including saved terminal facts and actual history after restart.
Saved-only results contain no fabricated Load ID, config or revision. Default
Turn inspection and explicit output ranges retain current execution semantics;
Turn inspect never exposes the full bound configuration.

`core_adapter.rs` performs the actual capability checks and owner queries.
Its private `core_adapter/inspect.rs` projections select the existing tool JSON
fields from those facts, including optional configuration and body reads. They
reuse owner state/history/partial values and do not query, retain results, or
introduce another public read contract. Full Turn results and current Turn
inspection remain separate projections because only the former can read
saved-only terminal facts after restart.

The tool layer re-exports core's factual Turn source and delegation registration,
disposition, completion, and input-range types. The input range serializes into
completion notifications; these facts do not grant authority. Opaque owner
capabilities, the separate port traits, and weak scope hooks retain the actual
acceptance checks and lifecycle boundaries.

The strict actions, snapshot configuration mapping, standard Runtime Tool,
selected ToolSet factory, ephemeral root scopes, pre-delivery registration,
independent completion delivery, and root cleanup coordination are implemented.
Startup composition retains its own continuation so dropping a timed-out tool
wait does not discard cleanup of an already created temporary Session.

Tests distinguish explicit owner-port fixtures from production owner tests.
`host_integration.rs` runs actual Host -> Agent -> Session -> Runtime -> agent
tool paths with only the model scripted: all six actions, independent completion
after the parent Turn and connection end, explicit source-config Agent creation
and temporary-owner reclamation, binding failure before input claim, and scope
close from the root's running tool. These are not live-provider or
TUI tests. Scope and contract tests separately cover forbidden targets, precise
config/range shapes, one-notification ownership and explicit cleanup retry.

The completion transfer reuses real owner TurnResult values. Notifications
select the last response's actual public text plus marked partial text and
outcomes; they do not copy all child history, reasoning, or tool traces. The
scope keeps only active relationships and payloads being transferred. Failure
of notification ends that attempt and is reported to its permit, without retry
or retargeting.

## Ordinary local-tool binding

`ToolRegistry::register_local_tools` registers the compiled read/edit/write/bash declarations from `minicore-host-local-tools`. It does not automatically select or enable them. Selected local tools require `BoundLocalTools::prepare` with the actual prepared Turn binding, a frozen `Workspace`, and a real `LocalToolInvocationSink`; ordinary `bind` explicitly rejects them without those resources.

The invocation adapter checks the existing Turn capability, receives a real owner-minted output lease, and calls the sink's synchronous `begin` before starting the worker. `ResourceResolver::bind_tool_output` must wrap selected implementations with the original `ToolOutputBinding`. Actual worker collection is registered with that public Turn, survives Runtime cancellation, and runs only at public Turn finalization. The real per-call sink completion closes the original lease after all file or process/group/pipe work has settled, so normal calls do not wait for Turn finalization. Native results enter history immediately; public Finished state and its single completion event require both the native result and confirmed actual cleanup, and carry final output positions. Internal errors with confirmed cleanup remain diagnostics; unconfirmed cleanup blocks finished. No standard agent action or delegation scope is changed.

`BoundLocalTools::production` constructs the real Session adapter used by `HostResourceBuilder`. The adapter holds only original invocation leases, byte offsets and incomplete UTF-8 code points. Edit observations enter the existing Session-owned journal; native ToolResults remain authoritative and use the existing history barrier. There is no second command-result store or tool scheduler. No production write uses `NoopLocalToolSink`.

`local_binding.rs` keeps the frozen local-tool wrapper, Turn cleanup registration,
Session recording adapter, and UTF-8 stream projection together. These are one
binding's execution and owner handoff responsibilities. `registry.rs` retains
the concrete Factory, BuiltinAgent, and Local registration facts; exact
declaration lookup does not bind a tool or grant a built-in exemption.

UTF-8 decoding preserves characters split across real chunks. A binary suffix stops text projection and marks that text range explicitly unavailable; existing prefixes remain unchanged. The successful native command result retains raw bytes. Leases are weak, expire only at actual tool cleanup, and cannot keep a reclaimed Session alive. Journal/process observations are not independently persisted or restored after restart.

The in-memory sink in `tests/local_binding.rs` remains an explicit test fixture, not durable edit capture or a production fallback. `host-resources/tests/local_tools.rs` covers the real production adapter, including the separate completion and process witnesses in `tests/local_tools/`.
