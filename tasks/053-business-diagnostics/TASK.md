# Task 053: Reconstruct business diagnostics

## Recovery boundary

The old environment and unpushed worktrees were lost. This is new implementation
and validation based on fixed recovered source
`c872e1d4d039684fa124d8b5a28fab8efc4ab11c`, not a claim that the complete prior
diagnostics implementation was restored byte for byte. Verbatim visible source,
patches, historical evidence and explicit gaps were saved separately under
`/workspace/shared/recovery-053` before this stage began.

The old production `6714259` passed 835 unique cases and 4 doctests; that is
historical evidence only. Its previous independent approval and integrated gate
do not validate this new tree. Missing functions and regression coverage must be
implemented and tested anew. SPEC v4.93, Runtime 0.10.0 at
`69faf9196991524b5549499d6208a1eaf567bf56`, TUI and dependency versions stay fixed.
The requested engineering configuration is gpt-6-astra/max, not an attestation of
the effective runtime. No worker push; the coordinator owns integration/backup.

## Initial file ownership

This slice owns provider production code and new provider diagnostics tests.
Transport is initially limited to an independent diagnostics module and pure
regressions. Its client/operations files and core/shared types remain coordinated
with task071. RPC/lib wiring requires a separate narrow handoff. No edits to
configuration, PreparedTurnBinding, local tools, Turn/query/history/core_adapter
or shared exports belong to this initial slice.

The six existing diagnostics/tracing dependency edges are already restored by
`f238626`; no manifest/lock edit is needed. Initial Cargo.lock SHA256:
`1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`.

## Provider reconstruction

`host-provider/src/diagnostics.rs` is copied from the complete source actually
printed before the loss, retained as `provider-diagnostics.complete.rs`. The
`lib.rs` request/response wiring is newly implemented against the recovered base.
The diagnostic is constructed on the actual request future's first poll, so an
unpolled start does not fabricate an attempted transport. A transparent stream
observer records only actual status, error kind/delivery, usage and monotonic
elapsed duration; drop is not success or cancellation. The provider keeps its
actual Runtime loop/request identity and explicitly clears unrelated Host spans.
No Model wrapper, retry, deadline change, body capture or stream transformation
is added. Runtime remains the only retry owner.

New tests use actual Runtime and the existing loopback HTTP/SSE fixture. They are
not live-provider validation and do not complete owner/HostInfo/RPC coverage.
Pinned Runtime has no source retry-wait elapsed measurement; it must remain
absent rather than inferred from observer latency or Retry-After.

### Provider validation checkpoint

The new compound regression covers actual Runtime retry after HTTP 429, original
loop/request IDs, known and absent usage, real HTTP 401, pending SSE drop, real
request cancellation, an unpolled start, unrelated-span isolation, and body/key
redaction. It also completes real work before installation and after the writer
has closed. The latter is a closed-output degradation check, not a disk-full or
filesystem-write-failure claim. No observations are registered on the Runtime.

Fresh commands use Rust 1.90, the locked offline dependencies, two Cargo jobs,
debug information disabled, incremental disabled, and this worktree's separate
target directory:

- `cargo test -p minicore-host-provider --all-targets --locked --offline`: 44
  unique tests passed (5 unit, 1 new compound regression, 38 HTTP/Runtime).
- `cargo clippy -p minicore-host-provider --all-targets --locked --offline -- -D warnings`:
  passed after removing a needless lifetime from the new test helper.
- `RUSTDOCFLAGS=-Dwarnings cargo doc -p minicore-host-provider --no-deps --locked --offline`:
  passed.
- `cargo test -p minicore-host-provider --doc --locked --offline`: 1 doctest
  passed. The all-targets `-- --list` run confirmed the 44 unique cases above.
- `cargo fmt --all --check` and `git diff --check`: passed.
- `cmp` with the rescued complete provider helper: identical. The new request
  wiring and tests are not claimed to match lost source.

The initial new regression had two fixture-only compile errors (the Runtime
attempt-count array comparison and construction of a typed test identity); the
first lint run then found the needless lifetime. Those outputs remain separate
from the successful reruns under `/workspace/shared/task053-recovered-provider-*`.
They do not imply that any historical lost gate was rerun successfully.

The entire workspace gate, independent review, owner diagnostics, HostInfo
health binding, transport integration, process/stdio and actual filesystem
failure regressions have not been completed in this reconstruction phase.

Fixed provider source: `037b24ab3e5af58b68ea6282656281aa9609f0d0`
(`feat(provider): reconstruct transport diagnostics`).

## Transport metadata preparation

The isolated `host-transport/src/diagnostics.rs` currently contains only narrow
correlation extraction. Its typed structs and borrowed, independent server
identity reads come from the rescued prefix. The existing-wire-byte adapter and
six pure regression tests are new. At this checkpoint it is included by the
standalone metadata test, not linked from the production crate. This is not
production RPC/client coverage and introduces no live logging call site.

Fresh focused validation passed: `cargo test -p minicore-host-transport --test
diagnostic_metadata --locked --offline` (6 pure cases), the same focused strict
Clippy target with `-- -D warnings`, `cargo fmt --all --check`, and
`git diff --check`. Actual transport call-site tests remain pending.

The server reads valid identity leaves independently, including initial-message
links and known requested targets. A malformed target must not erase another
valid original MessageId or in_reply_to. The wire-byte path accepts already
encoded bytes, skips unknown fields, and cannot reject or rewrite business
requests. No request is serialized solely for diagnostics. No body, config,
Role, key, path, or raw error field exists in the extracted structure. IDs
identify requested references, not proof that an owner published an object.

## Remaining owner and shared-hook plan

The new-base production inventory still has no core business diagnostic calls.
The existing app emits process bootstrap/shutdown/transport events and owns one
Guard; that does not cover direct-owner lifecycle, storage, Runtime outcomes,
or HostInfo health. The provider slice adds transport facts only; authoritative
retry severity, recovery and primary Turn failure belong to the Session owner.

After shared paths are released, the minimal health patch is a Host
`OnceLock<Health>`, read-only one-time binding, and `HostInfo.logging` obtained
from the existing atomic process snapshot. An unbound Host reports disabled
with unknown counters omitted. The application exposes a Health clone, never
the Guard, and binds it after real Host construction. Backend status types need
only Deserialize derives for the existing typed client response. Multiple Hosts
share the same process pipeline and counters; Host shutdown must not close it.

Owner diagnostic preparation and output remain separate. The direct owner
extracts a scalar Record from its committed fact, and its caller emits only
after the outermost state/admission fence is released. In particular:

- Input acceptance returns metadata alongside the original receipt through all
  public, accepted-creation and internal-notification paths. Queue binding uses
  the same committed revision as Turn startup, not the pre-publish revision.
- The first actual terminal claim returns the original failure metadata for an
  immediate primary ERROR; later failed history saves must not delay or replace
  that report. The finished summary waits for the existing real barriers.
- Actual history/metadata read/write workers record their own I/O failures.
  Queries waiting on an existing cold load do not invent another failed read.
- Creation failures are recorded by the actual creation owner when its failed
  receipt is claimed. Unpublished Agent/Session IDs remain absent. Child faults
  forwarded by Host do not become another principal ERROR. Ordinary resource
  validation refusal stays distinct from real Store failure.
- Tool process exit and wait/cleanup facts use the original ToolCall identity
  and actual CommandResult scalars after the existing adapter lock. A returned
  tool future alone does not prove process wait or cleanup.
- Cancel, scheduling, close, retirement, compaction and handoff diagnostics use
  actual accepted/completed facts without moving existing atomic save or
  lifecycle boundaries. No diagnostic queue, recovery entry, retry or extra
  lifecycle state is added.

Transport wiring must be coordinated across typed-local and RPC paths rather
than adding two client summaries or claiming RPC-only coverage is complete.
It needs a short operation span, existing request identities, and the original
returned error classification. Server summaries are forwarding metadata, not
another principal business ERROR. Client targets are target_* attributes, not
an invented local owner chain. Shared Operation machinery will not acquire a
generic logging framework and local calls will not serialize business requests.

## Shared-owner reconstruction checkpoint

The coordinator released the reviewed task066/070/071 production paths on fixed
`f0900a7083f9f6ddddbe978cd6ba55f4b1405c50`. A new isolated worktree continues from
that source. It already contains the provider slice; only metadata preparation
`a723c84f3130b609ee91abdaa9f06fcb9440a68e` was cherry-picked, as `9d3c377`.
Task054 fixtures remain outside this worker's ownership.

### Read-only process health

The first shared-owner slice adds only the planned Host OnceLock binding,
HostInfo logging snapshot, application Health getter/binding and Deserialize on
the public status value/enum. Health itself is not deserializable. The health
helper's first impl is restored from the saved complete Host diagnostic source;
the current shared declarations, app wiring and tests are new implementations.
Task071 discovery fields and task066 configuration/publication remain intact.

New actual tests cover three production-resource Hosts, bound versus unbound
counter knowledge, a real read-only stdio connection, Host shutdown/drop without
releasing the process writer's actual file lease, explicit application shutdown
of that writer, and refusal to replace a Host's first binding. A real application
child verifies host.info health through stdout RPC and preserves protocol-only
stdout plus safe stderr. These are health/ownership tests, not invented business
events or a claim of real disk exhaustion. The snapshot is obtained only from
existing atomics and never emits, flushes, takes writer locks or requires output.

Fresh health validation passed:

- `cargo test -p minicore-host-transport -p minicore-host --test diagnostic_health --locked --offline`:
  2 actual integration cases.
- The same two focused targets under strict Clippy (`-- -D warnings`).
- `cargo test -p minicore-host-diagnostics --all-targets --locked --offline`:
  21 unique backend/pipeline cases. Their fault-injection and subprocess fixtures
  keep their existing provenance; they are not claimed as new owner-path tests.
- Strict rustdoc for `minicore-host-core` and `minicore-host-diagnostics`, formatting
  and diff checks. Whole-workspace and independent review remain pending.

Fixed health source: `a6bec4a9757f264e968a98cffece5523272f7960`
(`feat(core): expose shared process logging health`).

### Typed and RPC call summaries

The next transport slice newly implements the missing Call completion/data
helpers and the current SDK/RPC wiring. It does not claim recovery of the lost
full transport file. Every actual client invocation emits at most one DEBUG
summary, after the original deadline-wrapped business call returns. The 47
explicit typed entrances and the existing control macro pass only known identity
leaves through a private field-reader callback, after that deadline is created.
The Operation trait/table, typed business execution and wire shapes are unchanged.

RPC reuses its existing encoded request bytes for narrow metadata reads, within
the same deadline. DEBUG-disabled calls skip this parse. The server reads leaves
independently from its existing borrowed parameters before validation, retaining
valid MessageId/in_reply_to even when a target leaf is malformed. Initialization
records only known connection identities, never the access key. Requested targets
are target_* attributes rather than a client-local owner chain. A requested Host
different from the connected peer cannot borrow that peer's incarnation.

Server dispatch and subscription setup each emit one forwarding DEBUG summary
after the actual result is available. The subscription request span/guard end
after setup, not at the end of the long-lived stream. Client timeout and abandoned
waits record unconfirmed delivery; neither claims cancellation, replay, failed
execution or a confirmed business receipt. A successful RPC response similarly
does not claim that the business outcome inside that response succeeded. Original
Fault type/code/diagnostic_id are forwarded without messages or details, and RPC
wrappers never manufacture a second principal ERROR.

Lock boundary audit: client output is after Operation::execute or RpcClient::call
and the complete deadline wait. RpcClient::request receives a borrowed diagnostic
and never emits while its pending-correlation mutex is held. Server output is
after initialize, dispatch or events_subscribe returns, outside their owner and
admission scopes. Drop summaries belong to the enclosing invocation, after the
inner business future and its guards have been destroyed. Spans use instrument
for futures or in_scope for synchronous setup; no entered guard crosses await.

New actual regression uses production resources/Host/Runtime, scripted HTTP,
typed calls and actual OS-pipe stdio. It checks initial creation message IDs,
requested Session targets, malformed-target -32602 with zero additional model
calls, original read-only Fault forwarding, timeout/drop without cancelled or
replayed business work, no observer dependency, and a new accepted/executed task
after the logging writer closes. The latter is known output rejection, not disk
exhaustion. Secret input, Role, model response, raw upstream error, key, endpoint,
invalid target text and arbitrary method text are absent from the JSONL.

The preparatory test-path wrapper was removed when diagnostics became a real
production module. Its six pure cases now run once as crate unit tests; two more
cover subscription scope and nested interaction load/operation identity. Initial
new-tree compile failures (scope field accessed as a method, a shadowed test
helper, and a diagnostic-only test assertion using the wrong BlockReason API)
are retained in separate local outputs. A first actual regression also timed out
because the fixture incorrectly expected a failed Session to schedule new work
without resume. It was corrected to invoke the existing public resume operation;
the correct production pause behavior was not changed.

Final focused validation on this slice passed:

- `cargo test -p minicore-host-transport --lib --test diagnostic_calls --test diagnostic_health --locked --offline`:
  13 cases (11 crate unit cases, including 8 metadata cases, plus 2 actual
  integration cases). The final dropped-wait exactly-once assertions passed.
- `cargo clippy -p minicore-host-transport --all-targets --locked --offline -- -D warnings`:
  passed on the final code, as did strict transport rustdoc, formatting and diff
  checks. No manifest, lock, SPEC, Runtime or task054 fixture edits were made.

The first full transport all-target attempt on the older f090-based fixture
stopped at `raw_result_and_wait_keep_complete_and_partial_indexes_across_internal_loops`:
`task054_query_contracts.rs:503`, after `client.close()`, the actual stdio server
join returned EPIPE (errno 32). Its business/query assertions had completed.
The full failed output is retained as
`/workspace/shared/task053-recovered-transport-all-targets.log`. This is not a
passing full gate. The coordinator identified the already independently reviewed
fix `5033e3b7bb316e6faf96a1362ca206013f58cde2` (fixture/private pipe helper only,
already integrated at root as `91cd50d`). It resolves the SDK Drop's second
unsubscribe reply racing the response reader close and keeps the original
business assertions. It will be brought in as a separate existing baseline
dependency after this functional slice is committed; it is not a diagnostics
production fix. No current workspace-wide or independent approval claim is made
for this slice.

Fixed transport source: `520d6f13993c67a2f22b966deb6fe4f892317170`.
The already-owned fixture dependency was then cherry-picked as
`55e3bcdd2af889898b9ecb0054cbd045525e563c` from source `5033e3b7bb316e6faf96a1362ca206013f58cde2`.
It must not be re-integrated at the root. On that fixed source, full transport
all-target tests passed: 138 unique cases, counted by `--list`. The green output
is `/workspace/shared/task053-recovered-transport-all-targets-fixed.log`; the red
pre-dependency output remains separate. This is not a workspace-wide gate.

### Owner record helper and first real Runtime consumer

The reconstructed core helper separates the owned scalar `Record(EventSpec,
EventData)` from emission. Its frozen crate-private API is `Record::new`,
`Record::emit`, `emit(IntoIterator<Item = Record>)`, `event`, `session_context`
and `turn_context`. It retains no owner, lock, queue, retry mechanism or body.
Callers must return these records through any outer admission fence and emit only
once all owner/observation/connection locks have ended. This is a small adaptation
of recovered source fragments, not a claim of a complete byte-identical old file.

The first consumer is the actual bound Runtime `RequestStarted` event in
`SessionController::observe_runtime`. It builds only the existing Turn identity,
LoopId, request index and response index under the existing Session boundary,
then emits after that lock and observation publication scopes end. This receiver
is called by the owned Runtime driver outside admission wrappers. It neither
synthesizes model attempts nor estimates unavailable retry-wait time.

The existing real Host/Runtime/local HTTP compound regression now checks all
three pre-close model starts against the actual completed/failed/resumed Turn
identities and the provider's original LoopId/request index. No observer is
installed. `cargo test -p minicore-host-transport --test diagnostic_calls --locked
--offline` and the same target's strict Clippy both passed; formatting and diff
checks passed. Logs are `task053-recovered-owner-helper-tests.log` and
`task053-recovered-owner-helper-clippy.log` under `/workspace/shared/`. Broader
owner lifecycle, Session acceptance/finalization/storage and tool diagnostics
remain unfinished and do not inherit a full approval from this helper slice.

### Review correction: initialization wait provenance

The independent task076 original `review076_initialize_timeout.rs` was replayed
unchanged. The red actually completed Host setup over OS pipes, gated its real
response, and returned the original 100 ms `WaitTimeout`; the logger incorrectly
said `wait_abandoned`. Its Call lived inside the original Deadline future and
was dropped before the outer boundary could observe that timeout. Red output is
`/workspace/shared/task053-initialize-timeout-red.log`.

The one Call now belongs to HostClient's existing stdio/WSS deadline boundary.
RpcClient initialization/authentication and WSS framing borrow it; only the outer
result finishes it. No new timer, attempt, request, authentication change, result
repair or owner state is introduced. A genuinely dropped constructor still logs
an unconfirmed abandoned wait. Unknown peer identities remain omitted.

The unchanged original probe passed after the fix, with green output separate in
`task053-initialize-timeout-green.log`. The permanent `diagnostic_initialize`
compound case covers stdio and authenticated loopback TLS/WebSocket, each with
an actual timeout and an actual task abort. A wire relay holds the production
Host's unchanged authentication response for WSS; it does not fabricate the
response or bypass certificate validation. The test checks exactly four client
summaries, the original timeout value, absent timeout on abandonment, no invented
peer ID, and no fixture credential in diagnostics. The existing real Runtime/HTTP
`diagnostic_calls` case also passed. The three focused executables (including the
unchanged review probe) passed. All-target transport strict Clippy, strict
transport rustdoc, formatting and diff checks passed. The temporary copied review
executable is retained outside the repository, not as a duplicate permanent test.
No full workspace gate or independent re-approval is claimed here.

### Remove redundant client diagnostic parsing

Review identified that the transport-local `tracing::enabled!` guard never
matches the installed diagnostics layer's record-module filter. The guarded
encoded-request metadata parse therefore contributed no production metadata.
Every private RpcClient entrance is already called with the outer typed Call:
Operation callbacks supply the actual scalar fields, subscription supplies its
typed scope, and initialization has no message/operation entity to extract.
The real installed-layer client tests already exercise this direct route.

The redundant byte parser, its guard, Deserialize derives used only by that path,
and its invocation after the original business encoding are removed. Server
metadata still reads each known leaf independently from the existing borrowed
parameters. There is no new filter API, request serialization or Operation change.
Two parser-only unit cases are replaced by direct typed metadata cases checking
original creation message links and tool target IDs without invented local
identity. An initial new unit fixture mistakenly included session_id in ToolCallRef
(which owns only Turn/tool leaves); its compile failure is retained in
`task053-remove-diagnostic-parse-tests.log` and the fixture was corrected without
changing production protocol types. Final 13 focused cases and all-target
transport strict Clippy passed (`task053-remove-diagnostic-parse-tests-fixed.log`
and `task053-remove-diagnostic-parse-clippy.log`); formatting and diff checks pass.

### Session input acceptance and binding

The original recovered scalar input helper is now used by the real Session
acceptance and binding boundaries. Public send and the already-admitted creation
send keep their public signatures. The private locked accept returns the existing
MessageAccepted plus its Record. Delegated input and final notification carry
that same Record through the existing Host execution/internal admission closure;
only the outer caller emits it, once the ConnectionRegistry and Session locks
have ended. No additional acceptance, body copy, queue, logger state or event
observer is introduced. Host/Agent creation must not log the same input again.

`observe_bound` returns scalar records rather than emitting. All three real
binding callers (initial Turn claim, same-Turn continuation and Runtime input
handoff) finalize their records using the revision produced by their existing
publish, release the Session lock, then emit. The small additive
`Record::published` helper changes only declared revision attributes. Initial
`turn.started` uses that same published revision and the actual Turn/Load identity;
no candidate Turn is reported as started. Public observation and original atomic
binding/save semantics are unchanged.

The actual Host/production resources/Runtime/HTTP test now asserts exactly three
accepted inputs, three bindings and three started Turns, with original IDs,
in_reply_to, real pre/post-dequeue lengths, no Turn on queued input, the accepted
receipt revision, and equal committed input_bound/turn.started revisions. It still
checks closed-pipeline business progress and private-content exclusion. This case
and focused strict Clippy passed. The 34 existing delegation/notification/deletion
fence tests also passed (6 delegation_acceptance_fences, 22 delegation_owners,
6 task051_deletion_fences); those retain their explicitly named gated model test
fixtures and are not called live provider tests. Outputs are
`task053-session-acceptance-tests.log`, `task053-session-acceptance-fences.log`
and `task053-session-acceptance-clippy.log`. Formatting and diff checks passed.
The remaining Session terminal/save/cleanup/close paths are not yet claimed.

The first later full core run found one private task071 fixture that directly
called `receive_delegated_owned` and ignored its new must-use Record. All core
business cases passed, but strict Clippy correctly rejected that unused record.
The fixture now emits its returned Record after the direct call returns (it has
no outer admission guard). Its original business assertions are unchanged. The
three task071 private cases and core+transport all-target strict Clippy pass after
this adaptation; the original warning/failure output remains in
`task053-terminal-core-all.log` and `task053-terminal-clippy.log`. This small
fixture adjustment belongs to the accepted-input signature, not a production
terminal-state change.

Second environment replacement: the preceding fixture change and task paragraph
are recovered from exact visible edit/append commands for old source
50a644ad552ae6c790ab67ce39e366bfefaa773d. The cited old local log files no longer
exist. Their outcomes above are historical, not a fresh validation claim. This
isolated reconstruction starts from verified remote 8c5fe493f9b9e78e15ef831c532bc267d0c7fb53;
new environment validation is pending. No root checkout or remote was modified.

### Immediate primary failure, real completion and history save failure

The existing `claim_terminal`/`claim_report` transition now returns whether this
call first set the outcome. Its owner extracts `turn.failed` only from that
first actual failed claim with an original business Fault. Every claim call site
carries the scalar record beyond the Session lock before emission, including the
report boundary that precedes a failed partial/terminal save. No logger claim
flag, diagnostic queue, second outcome, retry or save boundary is added. A
history-origin failure without a separate primary Fault keeps its storage ERROR.

`turn.finished` is INFO after the existing actual history, tool cleanup and answer
barriers, retaining the original outcome/Fault separately from persistence.
Its duration uses an Instant captured only at actual binding. The final records
use the committed Controller/Runner revisions. The run_turn future is instrumented
with only its known Turn identity outside all locks; no entered guard crosses an
await. `settle_save` extracts the original Fault, actual phase and retained Turn/
Load identity for the actual failed save attempt, then emits once outside its
lock. It neither retries nor converts a failed save into a business outcome.

New permanent `diagnostic_terminal_failures` uses production resources, a real
persistent Host/Store and loopback HTTP 401. The real opened history inode is
replaced while HTTP is gated, so the production Journal refuses the changed
identity. This is not a write-syscall errno or a full disk, and logging remains
healthy. A first fixture compile failure used the old query projection shape;
only its accessor was corrected. On unchanged pre-implementation production,
the actual red then confirmed finalizing/Failed 4204 plus persistence Failed 3301
but no primary ERROR before repair. Both outputs are retained separately.

The same test now passes: one primary ERROR before restoring the original inode,
no premature finished, existing explicit same-save resume, one model request,
original failed outcome/Fault retained, persisted final state, and exactly one
history ERROR and one final INFO. It checks no path, Role, body, response, key or
raw provider error enters logs. Final focused actual cases passed (terminal test
plus existing calls test), full core all-target tests passed 302 unique cases
(counted by --list), core+transport all-target strict Clippy and core strict
rustdoc passed, as did formatting/diff checks. Logs are
`task053-terminal-red.log` (fixture compile), `task053-terminal-red-fixed-fixture.log`
(actual red), `task053-terminal-green.log`, `task053-terminal-core-all-final.log`,
`task053-terminal-clippy-fixed.log`, `task053-terminal-doc.log`, and
`task053-terminal-core-list.log` under `/workspace/shared/`. The original lint
failure is recorded above. This remains a scoped reconstruction, not a current
workspace-wide or independent approval claim.

Second-replacement recovery note: the preceding terminal subsection is the exact
historical ledger append for source 5102c7e171e264869046b1b0bbcc682a831825ce.
The production edits, test and helper edits are replayed from visible literal
commands, then formatted with the restored official Rust 1.90 toolchain. The old
full raw logs and Git object are unavailable. No byte-identical final tree claim
or fresh test pass is implied. Cache restoration and new validation are pending.

### Close/delete admission bridge after the second replacement

The recovered caller patch from task079 has the exact original SHA256
`e439b94e2f45ba4cd9c8702d62cc371fc9b05fa6f50d83d5effd0f8b3b008c14`.
It changes only Agent metadata/collaboration/deletion and Host deletion call sites.
Task053 reconstructs the Session side from retained edit steps: close/delete
return scalar Records through those callers, which emit only after the outermost
existing admission closure has returned. The Agent cascade worker emits after
its child call, outside its state lock. No business boundary, result, save,
shutdown, retry or cleanup ownership is changed. Original delete operation IDs
are borrowed through the existing chain rather than replaced with new IDs.

`session.close_accepted` and `turn.cancel_claimed` come from the actual first
state transition. A cascade's later data-deletion claim does not repeat its
already accepted close/cancel. `session.closed` in this slice covers only an
actual immediate memory close, not unfinished asynchronous barriers. The helper
returns records without output in publish/observation or owner locks. The two
private direct-call fixtures consume these returned records after their calls.

The old final source `666abf29`, its complete final fixture diff and raw logs were
lost. The preserved caller patch is byte-identical, but the combined new tree and
reconstructed regression are not claimed to be the old complete commit. The
current deletion fixture's tuple/operation adaptation is newly validated. This
slice depends only on recovered `f4b105c`, not task079 creation/lifecycle changes.

Fresh validation on this combined source, with official Rust 1.90 and unchanged
locked offline dependencies:

- Actual production Host/Runtime/loopback HTTP tests `diagnostic_calls`,
  `diagnostic_terminal_failures` and `diagnostic_close_bridge`: all 3 passed.
  The new compound case covers Session close, Session delete and Agent cascade
  close, original operation/Turn IDs, exactly-once close/cancel/finished records,
  immediate idle memory Closed, and private-content exclusion. It registers no
  public observer. Its abort stops only the deliberately gated HTTP fixture
  after the real owner has completed cancellation; it is not provider success.
- `cargo test -p minicore-host-core --all-targets --locked --offline`: passed;
  the corresponding `-- --list` confirms 302 unique cases in this branch.
- Core plus transport all-target strict Clippy (`-- -D warnings`): passed.
- Core strict rustdoc (`RUSTDOCFLAGS='-D warnings'`, `--no-deps`): passed.
- `cargo fmt --all --check`, `git diff --check`, SPEC and lock hashes: passed.

The new raw outputs are `bridge-{focused,core,core-list,clippy,doc}-new-environment.log`
in the separately retained recovery reports directory. The earlier new-environment
terminal/calls focused pair passed, but its following core invocation was
interrupted during compilation; that truncated log is not a gate pass. The
successful bridge gate above also checks the recovered terminal code. No full
workspace or new independent approval is claimed here.

Remaining Session scope includes asynchronous close completion, scheduling and
other cancellation claims, actual tool/process completion and cleanup faults,
history loader/cold-read failures, compaction and result handoff. Pinned Runtime
still supplies no source retry-wait elapsed duration, so that SPEC requirement
remains explicitly unavailable rather than estimated or filled with zero.
