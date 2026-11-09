# Typed, stdio and authenticated WSS adapters

This crate connects the existing `Host` owners to a typed client and a trusted
stdio channel or TLS WebSocket connection. It is an incremental library, not a production Host executable.
The application still constructs the real resource resolver and opens its memory
or persistent Host. No fixture model or process configuration is installed.
Network listeners and TLS configurations are explicitly supplied by the embedding
application; network access is disabled until configured on the Host.

## Typed entry points

- `HostService` implements the protocol `MessageService<CallerContext>` directly.
  The context comes from an actual core `ConnectionHandle`.
- `HostClient::in_process(host)` registers one trusted embedding connection.
  Every business call uses the original Rust DTO and actual Host method. There is
  no JSON encoding/decoding on this path.
- `HostClient::stdio(reader, writer, options)` initializes one trusted channel and
  returns the same typed method results through JSON-RPC.
- `HostClient::wss(url, tls, access_key, options)` establishes authenticated WSS
  with one deadline covering TCP, verified TLS, WebSocket setup and initialization.
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
make it trusted; use `serve_wss` for the authenticated network boundary.

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
streams, complete production process/configuration loading,
live-provider operation, or TUI changes/validation. The transport tests use real
Host owners, real Runtime and real OS pipes/duplex IO with explicitly named model
and delayed-response fixtures. No external provider or TUI result is implied.

## Network authentication and key publication

`serve_wss(host, listener, Arc<tls::ServerConfig>)` serves actual TLS WebSockets.
It rejects startup unless `Host::configure_network_access(Some(key))` has supplied
a nonblank key. A missing key never grants anonymous access. This is a trusted
embedding hook, not a public JSON method. TLS certificates, listeners and client
trust roots are supplied explicitly; the library never loads credentials or
disables TLS verification. Plain `ws://` clients are rejected.

A WSS client sends one text JSON-RPC `initialize` request whose parameters are
exactly `{ "access_key": "..." }`. Even loopback must authenticate. Before that
request succeeds, no business reads, subscriptions or writes are dispatched.
Authentication plus registration share the Host connection acceptance boundary.
Invalid or unfinished initialization occupies no operation right; a reconnected
client is a new connection and never inherits rights. Configured secret state is
not serializable and its Debug implementation is redacted. Authentication failure
messages never include supplied keys or raw authentication input.

The trusted `Host::publish_network_configuration(caller, candidate_key, commit)`
hook is reserved for the future Host configuration owner. It checks current
operation rights and commits prepared configuration plus actual key invalidation
at the same narrow boundary as owner acceptance. The commit closure must only
check the prepared candidate version and publish state, with no I/O, child calls
or Host reentry. Candidate preparation/complete validation happens beforehand;
a candidate with restart-required changes must not call the hook. A failed
commit or unchanged key preserves existing connections. An actual committed
change revokes only old network connections immediately, regardless of when
socket cleanup runs. Trusted local/stdio connections and accepted owner work
survive. This hook is not a completed `host.reload` implementation.

WSS and stdio use the same request decoder, business dispatch, error/result
mapping and client correlation. WSS uses one complete text message per request
or response; fragmented text is not complete until its final continuation frame.
Binary application messages are rejected. Ping/pong and TLS/RFC framing rules
remain enabled. Both sides explicitly disable tungstenite frame/message size
limits and use unbounded application response queues; no application buffer
quota is added. Transport tests use generated localhost certificates, actual
TCP/TLS/WebSocket traffic and real Host/Runtime owners with named model fixtures.
They are not live-provider or CLI/TUI validation.
