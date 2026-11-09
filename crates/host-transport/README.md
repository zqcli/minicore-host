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
still applies, without adding transport text limits. EOF in a partial
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

`close()` ends both client directions; it is not an output-drain handshake.
If the peer closes its read end while a response or notification is being
written, `serve_stdio` can return `BrokenPipe`. An embedding that needs a clean
stdio server exit should finish its calls, close the request pipe, await the
server, then release the response reader. This does not add an observation ACK.

Requests dispatch concurrently. A slow create or `turn.wait` cannot serialize
unrelated controls/queries behind it. One writer owns each output stream; no
business-state lock is held while writing. Per-connection bookkeeping contains
pending response correlation and live observations, never a business operation/result store.

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
`host.reload` and `host.shutdown` carry their existing request DTO directly. Transport IDs never
replace `MessageId` or `OperationId`. `result` contains the whole actual receipt,
including accepted/start-error and partial-create facts, without an extra local
`Result` wrapper. A business rejection uses JSON-RPC `-32000`, with the complete
Fault in `error.data`. Registered leaves keep strict type/code/details checks;
newer unknown leaves retain their original type, code, message and opaque details
inside errors, receipts and queries. `Fault::kind()` is `None` for those leaves,
never a guessed known class. Unknown methods use `-32601`, malformed requests
`-32600`, malformed JSON `-32700`, and invalid parameters `-32602`. Canonical
methods without an adapter return `HOST_METHOD_UNAVAILABLE` (1401). Missing or
ambiguous retained receipts instead retain 1202/1203. Client-to-server notifications are ignored
and never dispatch `message.send` or other business work.

## Implemented routing scope

`SUPPORTED_METHODS` lists these 35 real routes:

- Host: `host.info`, `host.config.get`, `host.reload`, `host.receipt.get`, `host.shutdown`.
- Resources: `model.list`, `tool.list`.
- Agent: `agent.create`, `agent.get`, `agent.list`, `agent.stop`.
- Session: `session.create`, `session.get`, `session.list`, `session.state`,
  `session.load`, `session.unload`, `session.pause`, `session.resume`,
  `session.cancel`, `session.close`, `session.history.read`.
- Message: `message.send`, `message.get`.
- Turn: `turn.get`, `turn.list`, `turn.wait`, `turn.cancel`, `turn.result`, `turn.output.read`.
- Tool calls: `toolcall.get`, `toolcall.output.read`.
- Observations: `events.subscribe`, `events.unsubscribe`.
- Interactions: `interaction.answer`.

The count describes adapters for current core primitives, not completion of all
SPEC §8. Lists retain basic current-owner results. `turn.get` returns
`TurnSnapshot { source, data }` with the source flattened on the wire, using the
same atomic Session factory as `turn.state` and initial snapshots.
`session.history.read` takes `{ target, request: { cursor, limit } }` and returns
`SessionHistorySnapshot`; it reads real retained/cold persistent history without
creating a Runner or restoring old Turns. Text reads take the original Session,
Turn plus response index or ToolCall, and `request: { channel, start, end,
max_bytes }`. `end` is the fixed catch-up endpoint; all positions are UTF-8 bytes.
Missing/reclaimed sources return the real owner's Fault, not an empty success.

`HostClient::config`, `models` and `tools` use the same operation table and real
Host configuration owners on all three connections. `host.config.get` and
`model.list` take `{}`. `tool.list` takes `{}` for available tools, or
`{ "target": AgentRef }` / `{ "target": SessionRef }` for that owner's committed
enabled selection. `ToolListRequest` and `ToolListTarget` are the typed forms.
Mixed or incomplete references are rejected, and an Agent's Host qualifier is
never stripped. A Session may select tools outside its Agent's default list.

These queries are available to read-only connections. Results preserve complete
model references and original Runtime `ToolSpec` declarations in lexical order,
without new resource IDs, per-item kind tags, executors, or credential values.
Configuration contains credential source names, not resolved keys. An empty
enabled set is distinct from a missing target or a selected tool removed by a
successful configuration reload; those missing cases retain the owner's Fault.
Known references can be used directly without first calling either list method.

`HostClient::reload` and `host.reload` accept the canonical
`HostReloadRequest<HostConfig>` directly: `operation_id`, `expected_revision`,
and the full readable `candidate`. The initialization-time application preparer
comes from `HostResourceBuilder` or an explicit embedding. A Host constructed
without one returns `HOST_METHOD_UNAVAILABLE`; the adapter cannot install one.
Operation rights, lifecycle and revision/reservation checks stay at the actual
Host acceptance boundary. There is no `host.reload_config` alias.

The returned `HostReloadReceipt` is an immutable snapshot, normally `preparing`,
not a wait for publication. Later `host.receipt.get` reads the same configuration
owner's current `preparing`, `applying`, `applied`, `restart_required` or `failed`
facts. Preparation/save failures are retained in that receipt, while rejection
before acceptance is a business Fault. Duplicate operation IDs create separate
invocations and make a single-result lookup ambiguous; they are not replay keys.
No transport cache or automatic retry is involved. Caller timeout, disconnect,
or key invalidation cannot cancel accepted preparation/publication/cleanup.
The low-level prepared `Host::reload_configuration` embedding API remains valid.

## Shared observations and client content

`HostClient::subscribe(request, options)` returns a `ClientSubscription` with its
`SubscribeResult<StateEvent>` and asynchronous `recv()`. Only setup has the
ordinary call deadline. The established stream has no 30-second timeout.
`HostClient::unsubscribe` and dropping a subscription end just that observation,
without closing other streams, cancelling work, or retaining a Session/Runner.
An observation itself keeps its shared connection handle until dropped.
Read-only clients can manage only their own connection-bound observations.

The server uses standard JSON-RPC notifications:

- `events.push` has the existing `SubscriptionPush<ObservationEvent>` as params.
- `events.resync_required` has `ResyncRequired` as params, outside business events
  and category filtering. It ends only the named subscription.

The SDK separates notifications from responses, buffers early notifications by
subscription ID while setup is pending, and registers the returned baseline
without resurrecting an already ended stream. A late result from timed-out or
abandoned setup cancels only its actual registration. It neither closes the
shared connection nor repeats business execution. Old unsubscribe/resync frames
cannot affect a new subscription. Reconnection and resubscription remain explicit.

`OutputBuffer` is an optional client-owned helper for one exact source/channel.
It accepts the initial watermark, actual output/completion events and range-read
results. It retains late noncontiguous pieces, returns a fixed missing interval
with `next_read`, and merges overlapping UTF-8 bytes once. Completion before the
last body bytes is not treated as complete content. A source read Fault remains
visible to the caller; already received text is preserved. It does not silently
retry, resubscribe, retain source owners or invent missing content. Callers apply
state snapshots by the documented controller/Runner revisions, not output offsets.

Not implemented here: the remaining §8 routes, complete production
process/configuration loading, live-provider operation, or TUI changes/validation.
Tests use real Host/Runtime owners, actual OS pipes and loopback TLS with named,
gated model/tool fixtures. Ordering relays delay genuine Host frames rather than
manufacturing their business contents. Correlation unit tests are distinguished
from end-to-end transport tests. No external-provider or TUI result is implied.

## Network authentication and key publication

`serve_wss(host, listener, Arc<tls::ServerConfig>)` serves actual TLS WebSockets.
It rejects startup unless Host construction or the trusted
`Host::configure_network_access(Some(key))` embedding hook has supplied a nonblank
key. A missing key never grants anonymous access. TLS certificates, listeners and client
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

Canonical reload publishes the complete candidate and actual key invalidation
at one Host-owned boundary, only after preparation and necessary saving succeed.
Restart-required candidates do not partially apply. A failed candidate or an
unchanged actual key preserves authenticated connections and subscriptions.
A committed key change immediately revokes old network connections and their
operation right, regardless of socket cleanup timing. Trusted local/stdio
connections and accepted business work survive. The initiating WSS connection
may end before receiving its receipt; reconnect explicitly using the new key and
query the original operation ID, without resending the reload. The replacement
is a new connection with ordinary first-arrival operation-right rules.
The caller-checked `Host::publish_network_configuration` hook remains a low-level
embedding API, not an additional JSON method.

WSS and stdio use the same request decoder, business dispatch, error/result
mapping and client correlation. WSS uses one complete text message per request
or response; fragmented text is not complete until its final continuation frame.
Binary application messages are rejected. Ping/pong and TLS/RFC framing rules
remain enabled. Both sides explicitly disable tungstenite frame/message size
limits and use unbounded application response queues; no application buffer
quota is added. Transport tests use generated localhost certificates, actual
TCP/TLS/WebSocket traffic and real Host/Runtime owners with named model fixtures.
They are not live-provider or CLI/TUI validation.

## Exact interaction answers

`SessionClient::answer_interaction` accepts the original Turn, Load, public
20-character InteractionId, OperationId and a registered answer. Approval is
`{ kind: "approval", decision: "allow_once" | "deny" }`; tool input wraps the
Runtime's existing strict `ToolInputAnswer` as `{ kind: "tool_input", answer }`.
RPC parameters are `{ target: SessionRef, request: InteractionAnswerRequest }`.
The owner resolves the public ID to its original Runtime waiting point, validates
current operation rights and identities, and invokes that real slot exactly once.
An answered receipt means the slot accepted the answer, not tool/Turn completion.
It is retained under the existing operation receipt index.

Answers do not enqueue messages or clear pause. A new operator may answer a
still-pending old interaction. Wrong Load, other interactions, resolved waits and
late answers are rejected. Runtime answer-versus-cancel decides the actual fact;
the Session publishes answered only after Runtime acceptance. A private in-flight
fence holds final interaction settlement/reclamation until that synchronous
control call returns, with no Session lock held across the Runtime call or wait.
