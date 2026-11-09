# Typed and stdio adapters

This crate connects the existing `Host` owners to a typed client and a trusted
stdio channel. It is an incremental library, not a production Host executable.
The application still constructs the real resource resolver and opens its memory
or persistent Host. No fixture model, network listener or process configuration
is installed by the library.

## Typed entry points

- `HostService` implements the protocol `MessageService<CallerContext>` directly.
  The context comes from an actual core `ConnectionHandle`.
- `HostClient::in_process(host)` registers one trusted embedding connection.
  Every business call uses the original Rust DTO and actual Host method. There is
  no JSON encoding/decoding on this path.
- `HostClient::stdio(reader, writer, options)` initializes one trusted channel and
  returns the same typed method results through JSON-RPC.
- `client.session(SessionRef)` bundles only a target address and a shared client.
  It creates no registration ID, authority claim or server-side current Session.
  Different Session contexts can issue concurrent calls on the same connection.
- Cloning a client shares that connection. `close()` invalidates all its clones;
  dropping the last client/context closes it. This does not cancel accepted work.

Every ordinary call takes `CallOptions` separately. The default is 30,000 ms;
explicit overrides must be positive integer milliseconds. One Tokio monotonic
clock budget includes request preparation, writer queuing and the complete
response decoding. Neither the timeout nor caller authority enters business
parameters. Timeout returns the existing `CallError::WaitTimeout`; it removes
only that client's waiter. There are no automatic resends, rollback, execution
cancellation or shared-connection closure on one call timeout. Business failures
remain `CallError::Business(Fault)`; connection and protocol failures preserve
those separate error branches. A response arriving later is consumed under its
original transport ID and does not execute another request.

## Minimal framing and initialization

`serve_stdio(host, reader, writer)` serves generic async IO whose trusted origin
was established by the embedding application. Real OS pipe pairs and Tokio
duplex streams are tested. Passing arbitrary unauthenticated network IO does not
make it trusted; this crate does not provide a WSS authentication layer.

One UTF-8 JSON object occupies one LF-terminated line. Newlines inside JSON text
are escaped normally. CRLF is also valid JSON whitespace. There is no added frame
size, request count or send-buffer quota. Existing business/Runtime validation
still applies, including its pinned structural text bounds. EOF in a partial
frame is a connection error, not a complete request/response. Batch arrays are
not supported and receive `-32600`.

Before business calls, send exactly a JSON-RPC 2.0 request named `initialize`
with `{}` parameters and a string/integer transport ID. It returns the existing
`ConnectionInfo` directly. This is channel setup, not a 53rd resource method or a
new client identity API. No version/capability negotiation or authentication
claims are accepted in its parameters. A malformed or notification-only
initialize does not register a connection. Only the first valid initialization
on a channel calls `Host::connect`; duplicate initialization is rejected.

The first actual completed Host registration acquires free operation rights.
Existing observers remain read-only after that operator disconnects; a new
connection is required to acquire the freed right. Dispatch retains the original
`CallerContext` so each direct owner still checks validity at acceptance. EOF,
read/write error or dropping the server future closes its `ConnectionHandle`.
Accepted owner work remains independent of those transport tasks.

Requests dispatch concurrently. A slow create or `turn.wait` cannot serialize
unrelated controls/queries behind it. One writer owns each output stream; no
business-state lock is held while writing. Per-connection bookkeeping contains
only pending response correlation, never a business operation/result store.

For methods without parameters, send `{}`. Methods with an independent target
carry `target` plus that method's ordinary fields in `params`. For example:

```json
{
  "jsonrpc": "2.0",
  "id": "transport-17",
  "method": "session.pause",
  "params": {
    "target": { "agent_id": "<AgentId>", "session_id": "<SessionId>" },
    "operation_id": "<OperationId>"
  }
}
```

These placeholders illustrate field layout only; real IDs use the protocol's
validated ID format. `message.send`, `agent.create`, `session.create` and
`host.shutdown` carry their existing request DTO directly. Transport IDs never
replace `MessageId` or `OperationId`. `result` contains the whole actual receipt,
including accepted/start-error and partial-create facts, without an extra local
`Result` wrapper. A business rejection uses JSON-RPC `-32000`, with the complete
registered Fault in `error.data`. Unknown methods use `-32601`, malformed requests
`-32600`, malformed JSON `-32700`, and invalid parameters `-32602`. Canonical
methods without an adapter return `HOST_METHOD_UNAVAILABLE` (1401). Missing or
ambiguous retained receipts instead retain 1202/1203. Notifications are ignored
and never dispatch `message.send` or other business work.

## Implemented routing scope

`SUPPORTED_METHODS` lists these 25 real routes:

- Host: `host.info`, `host.receipt.get`, `host.shutdown`.
- Agent: `agent.create`, `agent.get`, `agent.list`, `agent.stop`.
- Session: `session.create`, `session.get`, `session.list`, `session.state`,
  `session.load`, `session.unload`, `session.pause`, `session.resume`,
  `session.cancel`, `session.close`, `session.history.read`.
- Message: `message.send`, `message.get`.
- Turn: `turn.get`, `turn.list`, `turn.wait`, `turn.cancel`, `turn.result`.

The count describes adapters for current core primitives, not completion of all
SPEC details for those methods. Query DTOs and available data are exactly the
current core's: lists do not yet provide public pagination, history is the
existing `Vec<HistoryItem>`, and current snapshots/results are not the complete
§9 event/output surface. Receipt lookup follows actual owner retention; closing
a memory Session may retire the lookup source immediately without invalidating
the already returned close receipt.

Not implemented here: remaining §8 routes, canonical event subscriptions/output
streams, WSS/key handling, complete production process/configuration loading,
live-provider operation, or TUI changes/validation. The transport tests use real
Host owners, real Runtime and real OS pipes/duplex IO with explicitly named model
and delayed-response fixtures. No external provider or TUI result is implied.
