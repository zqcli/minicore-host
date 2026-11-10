# MiniCore Host protocol foundation

Shared value contracts for the unchanged v4.93 design in `../../SPEC.md`.
This crate does not implement Host services, authentication, execution, storage,
transports, or TUI integration. A method's presence in the canonical catalog is
not a claim that its handler is implemented.

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
  must use checked increments. No epoch or DateTime encoding is invented here.
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

`PUBLIC_METHODS` and `METHOD_CATALOG` are the exact 52 resource methods in SPEC §8.
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
