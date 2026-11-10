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

### Actual tool completion and process facts

The existing native ToolResult/owned-output barrier now produces a scalar
`tool.finished` Record exactly when its original ToolCall first becomes Finished.
`observe_history` and `publish_cleaned_tool_results` return the records to their
existing callers. History acceptance, actual Turn tool cleanup, and the retained
output lease emit only after their Session lock ends. The production local
adapter already releases its map lock before calling the lease. No Runtime
ToolFinished observation is used to manufacture a result, no additional cleanup
or logging-state flag is added, and business history/save boundaries stay intact.

`ToolOutputLease::record_process_exit` is diagnostic-only and uses its original
Session-minted ToolCallIdentity. The one authorized `local_records::command`
call reads only CommandResult pid, optional exit code/signal and the distinct
actual process_waited/cleanup_confirmed facts, outside the adapter lock. It never
reads output or cleanup error strings, grants admission, confirms cleanup or
changes the command return. Unknown exit/signal values remain absent.

The new real Host/production resources/Runtime/loopback HTTP regression executes
bash with exit code 7, followed by a missing-file read. It compares the diagnostic
process fields to the actual native CommandResult delivered back through Runtime,
checks original Turn/tool IDs, exactly one completion per native result, unknown
signal omission, and no script/path/stdout/stderr/Role/body/key leakage. An ordinary
failed read remains a DEBUG tool result, not an infrastructure ERROR; the final
Turn completes normally. Approval is explicitly Auto because this is not an
approval test. No public observation listener or model/tool-result double is used.

Validation is new, not recovery of lost raw logs. The first fixture compile used
an obsolete Turn ID projection and was corrected to `done.source.turn_id`.
The corrected test on fixed e02f741 production failed with zero process records
rather than the required one, after all actual business/process assertions passed.
Both red outputs are retained. A subsequent run reused that red worktree's target
and mistakenly linked its old core artifact; it is excluded from validation.
The old rlib lacks the new diagnostic literal while the fresh one contains it.
Every different worktree now has a separate target; all green results below use
a completely new target dedicated solely to this source tree.

- Actual transport diagnostic focused tests: 4 passed (calls, close bridge,
  terminal/save dual failure, and tool/process completion).
- Core and tools all-target suites: 361 unique cases passed, 302 core plus 59
  tools, confirmed by the combined `-- --list` output. Existing fixtures retain
  their own explicit doubles and are not called live-provider validation.
- Core/tools/transport all-target strict Clippy and core/tools strict rustdoc:
  passed. Formatting and diff checks passed; dependencies and SPEC unchanged.

Current raw files are `tools-red-new-environment.log`,
`tools-red-fixed-fixture-new-environment.log`, the excluded
`tools-green-new-environment.log`, and successful
`tools-{green-isolated,core,module,core-module-list,clippy,doc}-new-environment.log`
in the recovery reports directory. No full workspace or independent approval is
claimed. Cleanup-fault, model retry/recovery, history-load, scheduling, remaining
close/handoff and compaction diagnostics are still separate work.

### Real history loading and Session metadata failures

`history_read` now records only an original load Fault returned by its actual
cold backend attempt, after the backend, admission and Session locks have ended.
It does not log an error propagated while waiting for an already-owned load.
The automatic preparation failure and explicit load worker each record their
own actual failed load once; the two explicit control call sites pass their
original operation ID. No candidate Turn or unpublished Load ID is fabricated.
Records formed before publish are finalized with the same committed revision.

The original Session metadata save completion returns one failure Record beside
its unchanged result. It retains the same candidate, baseline and original
Fault; explicit same-save continuation remains the only recovery mechanism.
The worker did not previously retain an initiating operation ID, so this slice
leaves that optional error field absent rather than inventing one. RPC/client
summaries still carry their actual operation. Successful persisted Closed and
actual memory close reached during load/preparation completion produce a Closed
record from the real before/after transition. These are not accepted-close facts.
No Agent propagated close Fault is upgraded into a second Session ERROR.

The new permanent production Host/Store/Runtime/local HTTP test is red before
this change and green afterward. It appends a malformed real history row, then
proves one cold-read failure without a Controller mutation or Runner, one actual
automatic-load failure retaining queued input, and one explicit-load failure.
After restoring the original bytes, existing resume executes the same accepted
message exactly once. Separately, replacing session.json with a directory makes
the production metadata boundary refuse a non-file; this is not disk exhaustion
or a claimed write-syscall errno. The original configuration/title remain live,
logging health stays OK, explicit same-save resume commits the original update,
and subsequent real persistent close produces one Closed record. The three
history failures and one metadata failure retain their original Faults and no
body, path, title, Role or key is logged.

Fresh validation in the same worktree's exclusive target:

- All 5 actual diagnostic focused cases passed (calls, tools, storage, terminal,
  close bridge), with the new actual red retained separately.
- Core all-target suite passed 302 unique cases, confirmed by `-- --list`.
- Core/transport all-target strict Clippy, core strict rustdoc, fmt and diff
  checks passed. SPEC, Runtime and dependency pins are unchanged.

Raw outputs are `storage-{red,green,remaining-focused,core,core-list,clippy,doc}-new-environment.log`
in the recovery reports directory. Full workspace and independent follow-up
review remain separate. Asynchronous memory-close paths outside these actual
completion points, other controls/scheduling, model summaries/recovery, tool
cleanup faults, result handoff and compaction diagnostics remain open.

### Runtime model retries, attempts and usage

The Session's already-owned Runtime event receiver now records actual
RetryWaiting StateChanged events as `model.retry_scheduled`. The fixed Runtime
source publishes this once from each real driver RetryWaiting update; the
separate watch-state projection does not produce a second log. Attempt 2 is WARN,
later attempts are DEBUG. No retry, deadline, state-dedup service or Model wrapper
is added. Only the original error mapping, actual attempt/max-attempt counters,
Loop/request identity and a supplied retry hint are recorded.

Per-request `model.finished` and successful retry `model.recovered` use the actual
joined LoopReport. The primary terminal claim is emitted first; report summary
extraction then runs outside the owner lock and before any later history wait.
Actual report model_attempts are paired with native Assistant/partial usage by
the report's original request index and the retained public Turn response base.
No potentially lagging watch count is substituted for the final count. Unknown
usage fields stay absent. A report without a complete response is explicitly
`no_complete_response`, not an invented failure/cancellation decision. These
summaries arrive when the existing Runtime loop returns; they do not create a
new completion barrier or copy/format any response body.

The new actual Host/Runtime/local HTTP regression was red on 7c7efd production
(zero model summaries after verified actual requests). It now passes across
three Hosts sharing the process logger: HTTP 429/429/success with known usage,
ordinary success with unknown usage, and actual bounded retry exhaustion. It
checks original Turn/Session/Agent/Loop/request identities, attempts 3/1/3,
WARN then DEBUG retries, one INFO recovery only for the recovered model, actual
usage normalization, no invented unknown counts, and exactly one primary ERROR
for exhaustion. No observer, model wrapper or fake tool/model history is used;
private bodies, Role, credentials and provider model text remain excluded.

Fresh results: all 6 actual diagnostic focused cases passed; the core all-target
suite passed 302 unique cases (new `-- --list`); core/transport all-target strict
Clippy, core strict rustdoc, formatting and diff checks passed. Raw outputs are
`model-{red,green,remaining-focused,core,core-list,clippy,doc}-new-environment.log`
in the recovery reports directory. SPEC and fixed dependencies remain unchanged.

**Known SPEC difference:** recovery requires actual waiting duration, but pinned
Runtime supplies only retry_at/retry_deadline, not source wait-start or elapsed.
The recovery event therefore omits duration. Neither a scheduled delay, retry
hint nor Host event-dequeue timing is falsely reported as measured wait time.
The new regression explicitly checks this omission. No Runtime change is made,
and this remains an acceptance limitation rather than a full first-version claim.

Next, preserve the known initiating Session metadata operation through its
existing save work and close/delete callers in a separate coordinated patch.
Other remaining Session control, asynchronous memory-close, cleanup-fault,
result-handoff and compaction diagnostics are still open.

### Original Session metadata operation across explicit continuation

This slice is based on fixed integration `587318b69a2e1f72d0f82999d3f6f6badacfe0e4`
plus `79cfc90b734ed55ef3541e4f29e8de344289e4e4`, the exact replay of model
source `0a7ed6254120868b7168ff02ae9479388b371665`. The earlier source branch is
retained. This worktree has a new exclusive target; no cross-worktree build
artifacts are reused.

The existing Session record work now retains only its optional original
OperationId. Public update supplies its actual request ID; direct Agent close
and delete workers pass the original operation already in their own work.
Explicit same-save resume/reconcile preserves that original scalar, including
an originally unknown identity. It never substitutes the later resume ID.
Candidate/baseline matching, storage calls, fault/result publication and recovery
remain unchanged. The completion still forms a scalar Record under the Session
lock and emits only after the lock and all caller admission guards have ended.
The legacy internal update API has no operation, so it still leaves it absent.

The three Agent caller edits are the exact task079-provided patch with SHA-256
`680218bba204f0703b151cb5be85c6ecc77413b67d0bdefaee744c3b5b51a839`.
Only the matching Session parameters, save metadata, existing deletion fixture
and new permanent regression are added here. No public API, new recovery entry,
span lifetime, retry policy or logging deduplication state is introduced.

The actual production Host/resources/Store regression first failed on the
missing operation field after all business assertions had passed. It replaces
the real session.json with a directory, separately exercising accepted update
and persistent close. Each failed save retains original 3301 and unchanged live
configuration. A second explicitly requested attempt while the same filesystem
refusal remains produces one further actual failure, still with the initiating
operation. Restoring the original file and using existing resume commits the
original update or Closed marker. The test verifies no Turn is fabricated and
no title, Role, key, framework instruction or path enters logs. This is a real
Store non-file validation refusal, not disk exhaustion or an asserted syscall
errno.

Fresh commands use Rust 1.90 and the unchanged locked dependencies:

- `cargo test -p minicore-host-transport --locked --offline --test diagnostic_metadata_operations -- --nocapture`: retained red before production changes, then green.
- Seven actual diagnostic focused cases: calls, close bridge, terminal failures,
  tools, storage, model and metadata operations all passed.
- `cargo test -p minicore-host-core --all-targets --locked --offline`: 317 unique
  cases passed, separately confirmed by `-- --list`.
- Core/transport all-target strict Clippy, core strict rustdoc, formatting and
  diff checks passed.

Raw outputs are `metadata-operation-{red,green,focused,core,core-list,clippy,doc}.log`
in the second recovery reports directory. These results do not claim full
workspace or independent approval. The coordinator is separately arranging
source-measured Runtime retry wait elapsed; this slice does not change its pin,
lock, or substitute an estimate. Remaining Session controls/cancellation,
asynchronous Closed, result handoff, cleanup-fault and compaction diagnostics
are still open.

### Public scheduling and explicit first-cancel boundaries

The actual pause/resume/load/unload acceptance closure now returns a Record
beside its unchanged receipt. A transient five-scalar before/after comparison
(pauses, residency and blocked fact) emits only a changed scheduling projection,
not a general state mirror or stored logging state. The record contains actual
queue lengths, original operation, existing Load/Turn identity when present and
the committed Controller revision. Repeated pause may still update its business
pause generation and receipt, but does not invent another scheduling change.
The existing load worker records its actual completion transition after publish
and unlock, retaining the same request operation. No new waits or mutations are
introduced for diagnostics.

Session.cancel and Turn.cancel now extract the existing first Claimed fact and
original operation before forming their receipts, then emit outside the entire
ConnectionRegistry acceptance guard. AlreadyEnding and NotActive do not produce
another claim. Runtime cancellation and original return values are unchanged.

The new permanent real Host/Runtime/gated local HTTP regression passed all
business assertions before its original red (no pause diagnostic). It now proves
paused accepted input remains queued without a model call, unload/load preserve
that input, resume publishes the same receipt revision before execution, and
both public cancellation forms claim exactly once. The actual Turn concludes
Cancelled and the upstream sees one original request, with no observers or
private content in logs. The first implementation build also caught the unused
Residency::Unloading enum arm; it was added explicitly and that failed compile
output is retained, not reported as a business red.

Validation: 8 actual diagnostic focused cases passed; core all-target 317
unique cases passed with a new list; core/transport all-target strict Clippy,
core strict rustdoc, fmt and diff checks passed. Logs are
`controls-{red,first-build,green,focused,core,core-list,clippy,doc}.log` in the
recovery reports directory. SPEC and locked Runtime remain unchanged. Agent
stop/force claim propagation, failure/save-driven scheduling, remaining actual
Closed/finalizing/handoff and compaction boundaries are separate pending slices.

### Actual asynchronous memory Closed completion

Six existing completion seams now compare the prior and published lifecycle:
driver return/resource-preparation return, joined Turn finalization, delegation
handoff return, scope cleanup completion, and the final scope-member release
signal. A transition Record is formed under the existing state boundary and
emitted after unlock. No close barrier, inbox disposition, callback order or
publication is moved; the logger still cannot close or release a Session.
Already Closed and unpublished owners do not generate another completion.

The permanent close/delete regression now also requires exactly one real
session.closed for each active memory Session after its original Turn finishes,
with a revision no earlier than the actual Turn completion. This addition was
red on 6a801fc (accepted close and cancelled Turn were present, Closed was absent)
and green with these hooks, for Session close, Session deletion and Agent cascade.
The existing idle-close assertion remains unchanged. The full core suite also
rechecks existing gated tool/scope/cleanup and deletion barriers.

Eight actual diagnostic focused cases and all 317 core cases passed, as did
core/transport strict all-target Clippy, core strict rustdoc, fmt and diff checks.
Raw evidence is `close-completion-{red,green,focused,core,clippy,doc}.log` in the
recovery reports directory. This does not claim all close/failure coverage:
compaction's locked completion requires its own later Record-return seam, and
result handoff/cleanup failures still need their separate scalar summaries.
Session.rs is frozen after this slice while the saved-result query P1 worker
coordinates its independent read-only trait/module change. No query/history-read
projection, SPEC, Runtime or lockfile changes belong to this slice.
### Isolated integration with Runtime source-measured retry waits

This is a **local integration candidate**, not a standard published-pin build.
The Host code in this slice requires Runtime fixed source
`2f7aeb9a2112e69cf1918c375916656e54d1208e` and its
`LoopReport.model_retry_wait_elapsed: Arc<[Duration]>` API. The coordinator
separately owns approval/publication and the eventual exact dependency pin.
Do not merge this code alone with the still-committed 69faf919 Runtime pin.

The only production change reads the same original request index already used
for model_attempts and complete response facts. Recovery includes that source
Duration as checked integer wait_duration_ms. Missing entries or unrepresentable
milliseconds remain absent. No Host clock, planned retry delay, Retry-After,
model wrapper, observer, additional state or wait is substituted. A normal
request still has no recovered event; retries scheduled do not prematurely log
elapsed waits. The prior SPEC acceptance difference is resolved by this candidate
only when paired with the new Runtime source, not in the published old-pin tree.

The real three-Host/Runtime/local HTTP regression was first red using the new
Runtime with old Host summary code: attempts/outcomes were correct but recovery
had no elapsed field. It is now green with measured elapsed, actual attempts and
usage, unknown usage absent, and bounded exhaustion retaining its primary Fault.
The retained measured run reports attempts=3 and actual wait_duration_ms=4;
this observed value is not asserted as a timing threshold or planned duration.

An external temporary Cargo config patched only minicore-runtime to the fixed
local Runtime checkout. Cargo's temporary lock differed by exactly the Runtime
source line; all registry versions/checksums remained fixed. After validation,
the original tracked lock was restored byte-for-byte (SHA-256
`1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`).
No path override, manifest/pin change, or temporary lock is included in this
commit, and the main Session topic was never modified for this experiment.

Local-config validation in its own exclusive worktree/target:

- Actual HTTP regression red then green, including the retained measured run.
- Eight actual diagnostic focused cases passed.
- Core all-target suite and new list: 317 unique cases passed.
- Core/transport all-target strict Clippy (rechecked after the scalar-only test
  output addition), core strict rustdoc, fmt and diff checks passed.

Evidence, exact external config, original/local-only locks and command outputs
are under the task053-local-retry-timing artifact directory. These checks do not
claim a standard published dependency gate, a full Host workspace pass, or
independent Host integration approval. No Runtime or Host push was performed.

### Published Runtime timing composition

The separately approved Runtime source 2f7aeb9 is published as
3aa47629929906832dcb309afb7d242c1ae3e53f on the existing
feat/host-history-handoff branch, parent 69faf919, exact tree
bb4cad715f52c96edaa95e9403416eb6ee619272. All 15 changed blobs/modes, raw commit
message, parent, actual author/committer zqcli and unchanged other refs were
verified through API and actual fetch. No Co-authored-by, force, public event
field or retry-policy change. Exact-head CI 42746444546 passed before this composition.

The Host consumer from e4be354 is composed with the published Git pin in all 8
existing manifest entries and the single lock source. No path patch or copied
Runtime is part of the normal build. Registry versions/checksums are unchanged.
Task089 independently approved the exact consumer against source 2f7aeb9, including
multi-Loop attempts/usage/indexes and dual execution/save failure. The following
normal published-pin tests belong to the coordinator's new integration checkpoint,
not retroactively to the earlier isolated experiment.

The normal `--locked --offline` production Git dependency build now passes the
three actual transport/model/terminal diagnostic regressions. The lock diff is
exactly the old/new Runtime Git source substitution, with every registry package
unchanged. Evidence: published-runtime-timing-focused.log in the coordinator
validation directory. The previous isolated candidate lock/config remains only
historical local evidence. A fixed composed full gate follows later.

### Actual delegation result handoff

The existing sink handoff boundary now extracts only the original child Session,
task message, originating parent Turn/tool identities and observed completion
counts/Turn IDs. After the real sink returns, the owner records its actual
success or fault and the final published Controller revision outside all owner
locks. No result body, tool output, registration dump or ended parent Turn is
reused as an active execution context. The original sink callback, handoff
ownership, pending/close facts and failure result are unchanged; there is no new
retry, acknowledgement, state flag or fallback.

A permanent real Host/Runtime/local HTTP regression runs actual parent and child
Turns with an explicitly named fixture delegation tool and gated fixture sink.
The parent finishes while transfer remains pending. Releasing the sink verifies
both successful transfer and its explicit independent failure, original task
and tool lineage, exactly one summary, and no private body/key/Role/raw fault.
The failure case retains its original close fault; it does not claim clean
shutdown or successful notification. The initial missing-summary red occurred
after the business assertions. A preceding fixture compile error for an unnamed
lock binding was corrected and retained separately, not counted as a red test.

Fresh validation on this exclusive worktree target: 9 focused diagnostic cases
and all 317 core all-target cases passed; strict core/transport all-target Clippy,
core rustdoc, fmt and diff checks passed. Logs are
`handoff-{first-build,red,green,focused,core,clippy,doc}.log` in the second recovery
reports directory. This is a handoff slice, not full workspace or independent
approval. SPEC, Runtime pin and Cargo.lock remain unchanged. Agent-stop claims,
remaining cleanup/failure scheduling and the separately owned compaction work
remain open.

### Agent stop retains the actual child cancellation operation

The coordinated thin patch (`688c927143f79a8c80c3f48f29aed72692901fa69ca8c95239fec071cebb6177`)
passes the original Agent operation through its two existing child enable/disable
callers. The Session extracts only its original first cancellable transition,
then emits after releasing its state. Both caller traversals already run outside
the Agent and admission guards. Repeated stop, already-ending Turns and the
existing deletion fence do not manufacture another cancellation claim. Runtime
cancel, queued inputs, receipt and Agent lifecycle behavior are unchanged.

The permanent real Host/Runtime/gated HTTP test reached Cancelled with one model
request before failing red on the absent claim. Green now proves one INFO with
the original stop operation and real Turn/Session/Agent IDs, and no second claim
for another stop request. Its two initial compile-only fixture mistakes (AgentRef
versus AgentId and the actual HostId source) are retained separately, not called
business failures.

Four affected actual diagnostic cases and all 317 core all-target cases passed;
core/transport strict all-target Clippy, core strict rustdoc, fmt and diff checks
passed. Evidence is `agent-stop-{first-build,second-build,red,green,focused,core,clippy,doc}.log`
in the second recovery reports directory. No full-workspace/independent claim is
made. The saved-result worker confirmed this Session setter does not overlap its
read-only HistoryBackend trait patch. SPEC and pinned dependencies are unchanged.

### Independent cleanup faults at the real finalizing boundary

The existing once-only tool cleanup owner now extracts one ERROR Record for each
actual returned independent Fault, after its handles have been awaited. Records
carry the original Turn/Load identity, actual outcome/persistence, cleanup
confirmation and published revisions. They are emitted only after Session unlock
and the existing release drop. The original primary Fault, cleanup_faults,
unconfirmed-work retention, result gates, returns and exactly-once cleanup claim
are unchanged. A logger does not confirm cleanup or rerun the tool.

A permanent actual Host/Runtime/OpenAI-compatible local HTTP regression combines
real HTTP 401 primary failure with an explicitly named fixture cleanup owner.
The fixture separately returns a confirmed independent issue or unconfirmed
cleanup. Both retain primary 4204, original cleanup diagnostic_id and one actual
cleanup invocation. The latter remains Finalizing without a finished event or
false result. The confirmed case finishes with its original primary and separate
cleanup facts. The unconfirmed case does not claim clean Host shutdown. This is
not a process wait, disk errno or real device failure injection. Existing actual
process cleanup tests also passed unchanged.

The corrected fixture first failed red on one ERROR instead of the required two
independent causes. An earlier fixture misuse attempted to abort an already
completed scripted HTTP server; its failure is retained separately and corrected
to join the real completed upstream, with no production change for that failure.
Fresh results: seven affected diagnostic cases and all 317 core all-target cases
passed, plus strict core/transport all-target Clippy, core strict rustdoc, fmt and
diff checks. Evidence is `cleanup-{fixture-failure,red,green,focused,core,clippy,doc}.log`
in the second recovery reports directory. Full workspace/independent validation
is not claimed. SPEC and pinned dependencies are unchanged.

### First force cancellation without invented operation identity

The existing Session shutdown traversal and Runtime input-fence boundary each
extract a Record only when they actually change Starting/Running to Cancelling.
Both emit outside Session state. The traversal uses its actual published
revisions; the input boundary did not publish, so it does not claim a new
revision. Neither fence, cancellation token, input disposal nor history behavior
changes. No Host or Agent caller signature/state is added.

These two callees do not receive an operation. It remains absent rather than
reusing an earlier graceful-shutdown operation during later force escalation.
This source limitation is explicitly approved and visible: the original
Turn/Load/Session and host_force reason remain available, but this event alone
does not identify which concurrent force request supplied the fence.

The permanent regression uses actual persistent Host/resources/Runtime, a gated
HTTP request and real history.jsonl. Graceful is upgraded to force, the original
Turn is saved Cancelled exactly once, and one cancellation INFO precedes the
actual finished summary. Repetition cannot claim again; no operation is guessed.
The missing-claim red was retained after correcting two test-only issues (an
unavailable storage dev-dependency import and the exact on-disk enum envelope).
No dependency was added. Five affected diagnostic cases plus all 317 core cases,
strict core/transport all-target Clippy, core rustdoc, fmt and diff checks passed.
Evidence is `force-{first-build,fixture-failure,red,green,focused,core,clippy,doc}.log`
in the second recovery reports directory. This is not full workspace or
independent review. SPEC and the original pinned dependencies are unchanged in
this source slice; the next worktree adopts the coordinator's separately
published Runtime and saved-result query baseline.

### Resource preparation disposition before Turn binding

The new worktree starts at coordinator integration
`8b129d4bdd35e223f8efe80059c4c8ea7bfe1df6`, including reviewed saved-result Option
APIs and the separately authorized published Runtime
`3aa47629929906832dcb309afb7d242c1ae3e53f`. Its exclusive target is distinct from
all previous worktrees. Source force `1fef32ca5d7ae7a670018afa26a22142e1ff5003`
was replayed once as `7bcc10e`; the coordinator must not integrate both copies.

At the existing final resource-preparation refusal boundary, the Session now
records its original typed Fault and actual queued counts/configuration revision.
Ordinary missing/configuration/lifecycle resources are WARN; unexpected failures
are ERROR. It also records the actual scheduling block with the same published
revision. No preallocated-but-unbound Turn ID, body, configuration or operation
is invented. Records form under the existing lock and emit after it; stale
configuration candidates still take the original continue path without changing
business behavior, accepted input or retry policy.

The real owner/HTTP regression uses an explicitly refusing ResourceResolver
fixture. Both expected missing-resource and unexpected failures retain the
original queued message without a Turn or HTTP request. Only the existing
explicit resume for a missing resource permits the original input to bind and
issue one actual request; the unexpected fault still rejects resume. The test
was red on missing resource diagnostics after all those business assertions,
then green without changing recovery behavior. No fake model or observation
listener is used, and private Fault/body/Role/key content stays absent.

Eight actual diagnostic cases passed, including source-measured retry elapsed on
the normal published pin, force, storage and primary/cleanup failure cases.
All 337 core all-target cases (new inventory), strict core/transport all-target
Clippy, strict core rustdoc, fmt and diff checks passed. The inventory is
`preparation-core-list.log`; raw outputs are
`preparation-{red,green,focused,core,core-list,clippy,doc}.log` in the second
recovery reports directory. Full workspace/independent review is not claimed.
SPEC is unchanged; this slice itself changes no dependency, pin or lock bytes.

### Internal delegated message rejection without a client observer

Existing tool.finished identifies a ToolCall/outcome, while handoff identifies
the original task message. Neither represented the distinct MessageId or original
in_reply_to of a rejected internal send/notification. The two actual outer
TurnCapability::send_delegated and DelegationPermit::notify returns now record
those request leaves with the unchanged Fault. No Host method signature,
protocol, input mutation, notification-once marker or business acceptance is
changed. Host/scope/admission/Session guards have all returned before emission.

The send uses its real bound source Turn; the target is a requested-target
attribute. A notification uses the actually registered child Session as source,
the parent as target attributes, and the old origin Turn/tool only as origin
attributes. It does not pretend the completed parent is a current Turn. No
missing Load/operation/owner is reconstructed. Ordinary rejection is WARN and
other call failures DEBUG; an underlying owner's primary ERROR is never repeated.
Original request in_reply_to is captured before the receiver's existing
notification normalization, so rejected calls are not described as accepted
rewritten input. Only scalar identities are retained for this invocation.

The real parent/child Runtime/local HTTP fixture now first runs without any
subscriber, then with the process pipeline. It rejects a nonmatching delegated
task ID before accepting the correct original task, and checks both failed
handoff notification and duplicate notification rejection after the parent ends.
Each rejected original message has one exact error correlation and no body/key,
while the original handoff and independent-fault assertions remain intact.
The additive test was red on absent message.send_error; its first compile-only
attempt incorrectly assumed Options was Clone and was corrected to move the
single pending installation. An earlier failed edit assertion left source
unchanged and only reran the baseline; that green run is not labeled a red test.

Six affected diagnostic cases and all 337 core cases passed; strict core and
transport all-target Clippy, strict core rustdoc, fmt and diff checks passed.
Evidence is `message-error-{unmodified-baseline,first-build,red,green,focused,core,clippy,doc}.log`
in the second recovery reports directory. No full-workspace or independent
review is claimed. SPEC, Runtime pin and Cargo.lock remain exactly the adopted
coordinator baseline. Remaining actual failure/save-resume scheduling summaries
will be composed separately with task088's reviewed original-Fault handoff.

### Actual failure and save-continuation scheduling differences

The final scheduling worktree is based on fixed coordinator
`50a1b2fedae73ce5c8b0a461a5bd6ed0edce68c8`, including the independently approved
compaction original-Fault handoff and its query-Option fixture composition. The
three preceding slices were replayed without conflicts or production rewrites:

- force `7bcc10e` (original `1fef32c`) -> `dcbfa00cd008a88fda61bfe0ef40d1c8b0f05c4b`;
- preparation `55bd353` -> `06bf117ad92090e9c69a0f4c6d28853f54bf45fe`;
- internal message errors `11e5bf5` -> `579edef82ed3e288a2b2786470e813c6037f667e`.

Only one copy of each patch belongs in integration. This worktree has its own
new target and unchanged adopted Runtime pin/lock.

The existing five-scalar Schedule comparison is reused at actual failure,
original history-save completion, explicit save continuation, metadata-save
completion and cleanup/preparation publication boundaries. Public control/load
summaries already covered requested scheduling changes; these additions cover
the distinct transition into failure-paused/blocked and the actual clearing of
that state after the original save is confirmed. No changed scalars means no
record. Normal per-token/internal phases are not added. Existing direct Fault
ERRORs remain separate and are not repeated by these DEBUG summaries.

Only immutable scalar comparisons and post-lock Records are added. The original
claim_report, ActiveTurn compaction failure slot, cancellation/save precedence,
atomic publication, callbacks, I/O, returns and recovery behavior are unchanged.
Known metadata/manual-compaction operation IDs are reused; the existing
history-resume worker retains no operation, so that scheduling summary does not
invent one or replace it with another caller's ID.

Permanent real HTTP 401 plus real history inode-refusal/explicit restoration
regression proves one failure-pause change, actual blocked true/false changes and
one completed resume. The cleanup fixture verifies only unconfirmed cleanup
creates a new block. Both additions failed before the new hooks. The metadata
regression keeps the original unpaused update/close cases and adds a genuinely
paused update: only the latter has a scheduling change on completion, carrying
the original update operation. An initial overly broad assertion expected a
change for the already-unpaused case; the failing output is retained, production
was not altered to manufacture that record, and the original case now explicitly
asserts no record. The added paused case establishes the real transition.

Eight affected diagnostic tests pass on the composed baseline, including actual
compaction utility/save failures, internal rejected messages/no subscriber,
source-measured retry waits, force and resource preparation. Red/green and
fixture-correction evidence is retained in
`scheduling-{failure-red,terminal-red,failure-green,focused-fixture-failure,focused}.log`
in the second recovery reports directory. The complete workspace gate and new
unique-test inventory are pending on the fixed code commit; no full gate or
independent approval of this final slice is claimed yet. SPEC/TUI/pins/lock are
unchanged by the slice. The separate saved-result read-failure check belongs to
task095, not to a new recovery or logging framework here.
