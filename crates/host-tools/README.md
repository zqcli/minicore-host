# Host tools

The statically compiled `agent` tool reuses Runtime `ToolSpec`, `Tool`, and
`ToolSet`. Its model inputs expose only the six actions in SPEC v4.93 §7.
Identity, operation/message ID generation, source capabilities, and configuration
revision selection are backend responsibilities, never model-supplied authority.

`TurnPort` is a narrow, Turn-bound owner integration boundary. Implementations
must use a trusted internal capability minted by Session before input claiming,
not a copied client `CallerContext`. Delegation permits and result sinks have
independent lifetimes, matching the registered scope and direct parent. The
adapter must receive necessary actual results before a memory child releases.

## Current stage

The strict actions, snapshot configuration mapping, standard Runtime Tool,
selected ToolSet factory, ephemeral root scopes, pre-delivery registration,
independent completion delivery, and root cleanup coordination are implemented.
Startup composition retains its own continuation so dropping a timed-out tool
wait does not discard cleanup of an already created temporary Session.

Tests use explicitly named owner-port fixtures, including one real Runtime
AgentLoop with a scripted model. They do not claim real Host delegation or a
live provider. The pending core adapter must implement these ports using the
new opaque internal owner capabilities, not a client connection.

The completion transfer reuses real owner TurnResult values. Notifications
select the last response's actual public text plus marked partial text and
outcomes; they do not copy all child history, reasoning, or tool traces. The
scope keeps only active relationships and payloads being transferred. Failure
of notification ends that attempt and is reported to its permit, without retry
or retargeting.

No remote push is authorized. No TUI files or SPEC are modified.
