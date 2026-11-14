# Typed, stdio and authenticated WSS adapters

This crate connects the existing `Host` owners to a typed client and a trusted
stdio channel or TLS WebSocket connection. It is an incremental library, not a production Host executable.
The application still constructs the real resource resolver and opens its memory
or persistent Host. No fixture model or process configuration is installed.
Network listeners and TLS configurations are explicitly supplied by the embedding
application; network access is disabled until configured on the Host.

## Source organization

The public entry points remain reexported from the crate root. The implementation
is grouped by the boundary each module adapts:

- [`src/client/connection.rs`](src/client/connection.rs) owns connection setup,
  `HostService`, and the typed local `Operation::execute` call path.
- [`src/client/host.rs`](src/client/host.rs) and
  [`src/client/session.rs`](src/client/session.rs) provide Host/Agent methods and
  the address-only Session context. [`src/client/subscription.rs`](src/client/subscription.rs)
  keeps observation setup and the public stream lifetime together.
- [`src/rpc/framing.rs`](src/rpc/framing.rs) handles complete frames;
  [`src/rpc/wire.rs`](src/rpc/wire.rs) defines request and notification envelopes;
  [`src/rpc/server.rs`](src/rpc/server.rs) initializes connections and dispatches
  requests to their existing owners.
- [`src/rpc/client.rs`](src/rpc/client.rs) keeps pending calls, setup buffering,
  the RAII waiter, and subscription cancellation association together. Its
  [correlation tests](src/rpc/client/observation_correlation_tests.rs) exercise
  early resync and cancellation after stream registration.
- [`src/operations.rs`](src/operations.rs) maps the protocol's existing `Method`
  variants to typed requests and real handlers. Streaming subscription setup
  retains its distinct path.

The query suites are [`tests/query_contracts.rs`](tests/query_contracts.rs) and
[`tests/query_recovery.rs`](tests/query_recovery.rs). Their
[drop cleanup](tests/query_contracts/drop_cleanup.rs) and
[retained cleanup](tests/query_recovery/retained_cleanup.rs) scenario modules
preserve separate writer gates: one is explicitly armed before SDK Drop, while
the other selects the actual `ended=false` cleanup response. Both reuse the
existing [pipe fixture](tests/common/pipes.rs). Diagnostic integration binaries
remain separate so each can install its own process-global subscriber.

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

`SUPPORTED_METHODS` contains the 55 SPEC §8 resource routes:

- Host: `host.info`, `host.config.get`, `host.reload`, `host.receipt.get`, `host.shutdown`.
- Resources: `model.list`, `tool.list`.
- Host directory selection: `workspace.roots`, `workspace.list`, `workspace.mkdir`.
- Session workspace: `workspace.files`, `workspace.read`, `workspace.search`, `workspace.status`.
- Changes: `changes.list`, `changes.diff`.
- Agent: `agent.create`, `agent.get`, `agent.list`, `agent.update`, `agent.start`,
  `agent.stop`, `agent.set_default_session`, `agent.delete`.
- Session: `session.create`, `session.get`, `session.list`, `session.update`, `session.state`,
  `session.load`, `session.unload`, `session.pause`, `session.resume`,
  `session.cancel`, `session.close`, `session.delete`, `session.history.read`,
  `session.context`, `session.compact`, `session.compact.cancel`.
- Message: `message.send`, `message.get`.
- Input: `input.list`, `input.remove`.
- Turn: `turn.get`, `turn.list`, `turn.wait`, `turn.cancel`, `turn.result`, `turn.output.read`.
- Tool calls: `toolcall.get`, `toolcall.output.read`.
- Observations: `events.subscribe`, `events.unsubscribe`.
- Interactions: `interaction.answer`.

The list describes actual routing, not complete product acceptance. Ordinary
operations share `src/operations.rs`; `events.subscribe` uses the real client/RPC
subscription path. Lists retain basic current-owner results. `turn.get` returns
`TurnDetailsSnapshot { source, data }` with the actual Turn identity flattened
on the wire. Its data combines the same authoritative `TurnStateData` used by
`turn.state` with the complete original bound Session configuration; later
Session updates do not replace it. State subscriptions stay lightweight.
`message.get`, `turn.list`, `turn.result` and `turn.wait` also return actual
Host/incarnation/target identity and a `data` envelope. Turn lists preserve completion
order followed by the active Turn. Results project complete native history and
separate unfinished partials, exposing actual public response indexes but no
Runtime Loop IDs, request indexes or provider replay payloads. These snapshots
read the existing direct owner and do not extend memory retention.

After persistent reopen, `turn.result` and `turn.wait` can project saved terminal
content without loading or restarting execution. Retained results contain
`data.info` and `data.config` with their original LoadId; saved-only results use
`data.saved_terminal` instead and omit LoadId. Invalid mixed/missing branches
are rejected, not filled with invented configuration or counters. See
`tests/task087_saved_result_contract.rs`.

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
enabled selection. `ToolListRequest` and `ToolListTarget` are protocol query DTOs, reexported here
for the existing typed API.
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

## Manual compaction

`SessionClient::compact(CompactRequest, options)` and
`SessionClient::cancel_compact(CompactCancelRequest, options)` route directly to
`Host::session_compact` and `Host::session_compact_cancel`. Independent RPC calls
use the same flat target and operation fields as other Session controls:

- `session.compact`: `{ target: SessionRef, operation_id: OperationId }`.
- `session.compact.cancel`: `{ target: SessionRef, operation_id: OperationId,
  target_operation_id: OperationId }`.

The cancellation has its own operation ID and explicitly targets the original
manual operation. JSON-RPC IDs only correlate wire calls; no operation ID is
replaced or generated at the server. Unknown fields, missing targets and invalid
IDs are rejected. Caller authority and wait timeouts are not business parameters.

The actual Session owner checks current connection rights, lifecycle, history
availability and competing Turn/compaction work. An accepted manual operation
does not create a Turn or queue a message. Its utility work, history commit and
applicable saving continue independently of the client waiter and connection.
Messages still enter the two existing queues; completion or cancellation neither
clears those queues nor clears explicit pause. Other Sessions continue normally.

The returned `CompactReceipt` is a fact snapshot, usually `preparing`, and
`host.receipt.get` retains that original receipt. Later progress is available in
`session.state` and its existing state events, including the original manual
operation ID and real utility outcome. Those events can arrive before the call
receipt without changing what that receipt means. `CompactCancelReceipt` reports
whether this request claimed cancellation and the target's then-known facts;
it does not promise immediate stop or rollback. A late cancel of an older
operation never targets a newer compaction or Turn. Missing original operations
return the owner's Fault rather than a guessed result. Timeout, aborted caller
and connection loss never automatically cancel or replay accepted compaction.

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

Production process/configuration loading is assembled by `host-app`, not this
transport library. Live-provider operation and actual TUI integration remain
outside the validation described here.
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

## Workspace and retained edits

`SessionClient::workspace_files`, `workspace_read`, `workspace_search`,
`workspace_status`, `changes_list` and `changes_diff` route through the explicit
Session's direct owners. All are available to read-only clients. RPC parameters
are `{ target: SessionRef, request: <existing host-workspace request> }`; status
takes only `{ target: SessionRef }`. Returned `WorkspaceSnapshot<T>` flattens
the actual Session identity and puts the existing backend result in `data`.
Filesystem/Git results include the committed `config_revision` that selected
the root. Session/Turn tool observations have no current configuration revision.

Workspace reads clone that Session's committed configuration before binding the
root and performing I/O outside owner locks. An active Turn keeps its original
workspace after Session configuration changes; queries never borrow that Turn
or a client-selected current page. Git changes are current workspace facts with
`workspace_unknown` origin. Their existing `ChangeRef::Workspace` includes a
`workspace_scope` fingerprint of the original canonical root. Diff rejects a
different root with `SESSION_CHANGE_NOT_FOUND` before Git/content reads, even
when HEAD and status happen to match. Two Sessions using the same root can read
that reference; this source marker is not an authorization credential. Session/Turn changes come only from the Session's
actual captured tool observations, and diffs use their exact before/after bytes
without reading a current file to fill missing history.

The backend's UTF-8 byte ranges, explicit page sizes, bound continuations, local
path containment and external-filter rejection remain unchanged. Invalid
ranges/cursors, missing sources, unsupported content and I/O failure are public
Faults, not complete empty pages. Unload retains existing observations. Source
reclamation removes them; after persistent reopen, Session-wide old edit
coverage is explicitly missing while new retained Turn observations remain
queryable. There is no additional durable edit database.

`CallOptions` remains a client wait budget, not a workspace or business execution
deadline. It is never serialized or connected to the active Turn's cancellation.
The low-level Host methods accept the backend's optional `QueryContext` for an
embedding's own query cancellation; that context also cannot cancel a Turn.

## Explicit owned-data deletion

`SessionClient::delete` and `HostClient::delete_agent` share the same direct-owner
admission and cleanup through typed calls, stdio and authenticated TLS WSS.

- `session.delete` takes `{ target: SessionRef, operation_id, close?: boolean }`.
  Omitted/false `close` requires an already closed, settled Session. True requests
  normal close, required result/save/tool cleanup and then only its owned data.
- `agent.delete` takes `{ target: AgentRef, operation_id, scope? }`. Omitted scope
  is `{ "kind": "empty" }`, requiring a stopped Agent with no retained Sessions.
  Explicit cascade is `{ "kind": "sessions", "session_ids": [...], "close": true }`.
  IDs must exactly cover the current direct children; omission of `close` does
  not authorize stopping/closing them. Unknown fields/duplicate IDs are rejected.

The control receipt is formed when its actual owner accepts and fences the
work. It never claims that files have already been removed. Agent get and
Session state expose the corresponding current deletion work, including partial
failure. Later progress never rewrites the receipt. Success removes the actual
source, so subsequent source reads return not-found. A Session delete receipt
belongs to its Agent; an Agent delete receipt belongs to Host, subject to those
owners' real lifetimes. Other child-owned receipts are not copied to a cache.

Cascade deletion never follows physical nesting as authorization. Root scope
close may settle registered descendants through their real owners while their
persistent directories remain intact unless explicitly included. Received
launch ToolResults/completion messages and independent workspace artifacts are
not deleted. A filesystem error may follow partial removal and is reported as
such, without rollback, implicit retry or fabricated full deletion.

## Directory selection before Session creation

These methods are available through `HostClient` and the same stdio/WSS operation
handlers. `workspace.roots` takes `{}` and returns `{roots:[{name,path}]}`.
`workspace.list` takes `{path,hidden:false,limit,cursor:null}` and returns
`{path,parent,entries:[{name,path}],next_cursor}`. The optional cursor is
`{scope,after_name}`; page size must be positive and can change between calls.
Only real direct child directories inside configured `workspace_roots` are listed;
returned paths are canonical absolute Host-platform paths. No Session reference is
accepted or required. The client can pass a selected path as `WorkspaceRef.path`.

`workspace.mkdir` takes `{operation_id,parent,name}`. Only the current operator can
submit it. It creates exactly one child and returns the standard Host receipt with
`kind:"workspace.mkdir"` and `result:{status:"created",path}` or
`result:{status:"failed",error}`. `host.receipt.get` may observe
`result:{status:"creating"}` while the original owned worker is active. Caller
timeout or disconnection never cancels the write. Existing entries fail, and an
operation ID does not deduplicate invocations. All reads require a valid connection;
read-only peers do not gain mkdir rights when the operator disconnects.

Host directory errors are registered at 1701–1709: invalid path, outside roots,
not found, not directory, already exists, invalid page size, invalid cursor,
filesystem failure and unsupported path encoding. They retain the ordinary Fault
and JSON-RPC error/receipt placement; underlying OS diagnostics are not exposed.
