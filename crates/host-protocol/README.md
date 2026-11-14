# MiniCore Host protocol foundation

Shared value contracts for the unchanged v4.10.1 design in `../../SPEC.md`.
This crate does not implement Host services, authentication, execution, storage,
transports, or TUI integration. A method's presence in the canonical catalog is
not a claim that its handler is implemented.

## Source organization

Public value types remain available from the crate root. Functional families
keep their requests, facts, and receipts together:

- [`connection.rs`](src/connection.rs) contains caller wait options and actual
  connection/control facts, separate from business controls.
- [`host_control.rs`](src/host_control.rs),
  [`agent_control.rs`](src/agent_control.rs), and
  [`session_control.rs`](src/session_control.rs) contain the respective owner
  controls. Agent start and stop share one module.
- [`metadata.rs`](src/metadata.rs) contains Agent/Session metadata updates and
  their shared save/commit result; [`input.rs`](src/input.rs) contains the two
  input queues and explicit input removal.
- [`compaction.rs`](src/compaction.rs) contains manual compaction and cancellation
  requests alongside their progress, result, and receipt types.
- [`resources.rs`](src/resources.rs) contains `ToolListRequest` and
  `ToolListTarget`. The transport crate reexports these same types.
- [`catalog.rs`](src/catalog.rs) remains the sole method-name catalog. Transport
  operation definitions refer to its `Method` variants.

These module boundaries do not add protocol identities or change JSON layouts,
root exports, validation, or the direct owners of business state.

## Invariants

- MiniCore IDs are distinct Rust newtypes, encoded as exactly 20 lowercase symbols
  from `0123456789abcdefghjkmnpqrstvwxyz`. `new_id()` and `Type::new()` return errors
  when the OS CSPRNG fails. There is no weak fallback, prefix, sorting meaning or
  application deduplication service. Upstream ToolCall IDs remain unchanged.
- References carry only their immediate owner. Session mode inherits its direct
  Agent when omitted; a memory Agent cannot own a persistent Session.
- `Revision`, `ConfigRevision` and `ControllerRevision` use unsigned `u64` integers
  in Rust and JSON, not IDs or timestamps. Consumers must retain integer precision
  and compare only within the documented Host incarnation and owner scope. Owners
  must use checked increments. Wall-clock metadata is separately encoded by `Timestamp` as UTC RFC3339;
  comparison uses chronological instants, with input offsets normalized to UTC.
- Requests carry text, message correlation and a target, never CallerContext,
  operation rights, attachment payloads or caller waiting options. Paths are not
  read or rewritten by message contracts. Empty text is structurally a string;
  an actual Runtime input incompatibility must be reported by its real owner.
- `MessageAccepted` describes acceptance, actual queue and already-known startup
  facts. It never implies a bound Turn, successful execution or saved history.
- Create receipt identities exactly match published objects. Earlier successful
  steps survive later failure. Message fields appear only for a requested message
  and acceptance facts only for a succeeded message step. Progress queries may
  contain pending/running; `validate_call_return()` rejects them in ordinary
  creation-call returns.
- Fault type/code pairs and explicit classification are registered centrally.
  Details are typed and limited to relevant branches. Retry hints are not retry
  permission. `CallError` preserves client-side timeout/connection/protocol
  provenance rather than manufacturing server facts.

## Configuration

Effective `AgentConfig` and `SessionConfig` are complete values. Fields reused by
SessionDefaults, creation overrides and update patches have identical types.
Patches replace complete structures and arrays; an empty tool array clears tools.
Only `workspace` accepts null to clear. Omission on creation selects the captured
Agent default; omission on update retains the current independent Session value.
Candidate construction does not mutate the original, and invalid in-process
configurations fail serialization too.

`AgentConfig.instructions` and `SessionConfig.instructions` are optional text
(default empty), alongside `Role.instructions`. They use the same name in
creation overrides and update patches; omission retains/selects the owner value,
empty text clears it and null is rejected. Directory loading resolves the Role
reference before owners receive native configurations. Agent declaration keys
are optional owner `configuration_source` metadata, never configuration fields.
`HostReloadRequest` contains only `operation_id` and `expected_revision`; its
result preserves per-Agent facts and an explicit `partially_applied` state.

The `default` model in `Default` implementations is an unresolved selection
reference, not evidence of a configured provider. `ConfigResources` validates
actual Host references without sending model requests, executing tools, replacing
resources, changing process cwd or granting permissions. Production owners must
perform this check and appropriate version/state checks before publishing values.

Supported execution settings include Runtime timeouts/retry values, compaction
and per-Turn tool approval. `execution.approval` defaults to `ask`; `auto` and
`read_only` are explicit alternatives. No event capacity, tool-round cap, entity
limit, input-size limit or application quota is introduced. The inherited Runtime ModelRef grammar
(1–256 bytes) and model retry/deadline legal ranges are documented constraints of
that dependency, not new Host admission controls.

## Calls and staging

`MessageService<Caller>` places the trusted owner-created context outside the
request. `MessageClient` takes caller-side `CallOptions`, with a positive integer
millisecond deadline defaulting to 30000. The client times the entire wait once
using its monotonic clock. These value contracts do not cancel work, retry writes,
rebind old connections, or claim that a stopped waiter stops accepted execution.

JSON-RPC message.send params are exactly SendMessage. A normal result contains
the full receipt, without a serialized Rust Result wrapper. `RpcResponse<T>`
rejects simultaneous result/error. Business Faults are error.data and retain their
own code separately from the JSON-RPC code. Timeout/connection errors remain local.
Owners construct registered Fault leaves. Deserialization also preserves newer
unrecognized peer leaves and opaque details; `kind()` then returns `None`.
Registered type/code/details combinations remain strictly validated.

`PUBLIC_METHODS` and `METHOD_CATALOG` are the exact 55 resource methods in SPEC §8.
Host state events are not a new `host.state` method. The catalog describes canonical
names, interaction forms and access requirements; the transport router reports
its supported methods. `events.unsubscribe` requires ownership of the observation
rather than Host business-operation rights.

The obsolete implementation-stage Rust API (`ImplementationStage`, `planned_stage`,
`MethodAvailability`, `MethodCapability`, `staged_catalog`, and `require_implemented`)
has been removed. The registered `HOST_METHOD_UNAVAILABLE` Fault remains available
for a canonical method without an active handler.

## Validation and limits

See `../../tasks/002-contracts/TASK.md` for commits and exact commands. Tests cover
ID grammar and unbiased encoding; ownership/mode/config rules; invalid request and
receipt combinations; Fault registry and typed details; connection/control facts;
method completeness; and local/RPC wire equivalence. These are contract tests, not
live provider, Runtime, storage durability, WSS or TUI acceptance tests.

This crate is a member of the root workspace and uses its committed `Cargo.lock`.
No standalone workspace, lockfile or local Runtime patch is required.

## Stable history observation

`HistoryReadRequest` carries `cursor`, positive `limit`, and optional `direction`
(`forward` by default, `backward` for tail-first reading). `HistoryPageData` returns
`items`, `next_cursor` and the captured exclusive `upper_bound`. Each page is
oldest-to-newest, and later appends do not enter the fixed pagination window.
Cursors bind the actual Session, Host incarnation and requested direction.

Core history records expose original native positions and applicable Turn/response
identity, with UTC owner timestamps. Bound-input and completion events and saved
Turn results use the same positions. Unbound inputs and cold saved results do not
acquire fabricated record positions or LoadIds. Host object schema 2 and history
schema 3 retain these facts; old envelope versions are rejected without migration.

`DirectoryRoots`, `DirectoryListRequest/Result` and `DirectoryMkdirRequest/Receipt`
are Host-level directory selection contracts. They do not carry Session IDs.
`workspace.mkdir` uses the existing control receipt envelope and Host Fault registry.
