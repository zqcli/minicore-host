# Host 0.11 history and activity implementation

## Scope and baseline

Implementation starts after design-first commit
`32ea194ff2269aa8fb2bebff79355b600d404b45` (SPEC 4.10.2), on
`work/history-0.11`. Requested worker configuration was `gpt-6-astra` with `max`
reasoning; effective runtime settings were not independently verified.

This worker owns core/storage/protocol history contracts, Session activity,
related observation projections, component documentation and affected fixtures.
Root owns model API configuration, release versions/lockfiles, integration,
publication, and the main task/project records. No TUI changes, argument-generation
observation, old-format migration, new RPC methods, quotas, implicit retries or
old execution recovery are introduced.

## Implemented contracts

- Public history wraps every accepted native record in its original logical
  `position`, `recorded_at`, and applicable `turn_id` / `response_index`.
  Explicit terminal facts retain `finished_at`, request and tool-round counts,
  actual attempts, outcome and reason. Compaction markers retain source position
  and cumulative native coverage. Duplicate MessageIds do not identify records.
- Input acceptance captures `received_at`; delayed binding retains that instant.
  Bound-message and complete model/tool events reuse accepted record positions
  and times. Frozen terminal state/events and retained results reuse original
  terminal times; cold results still omit LoadId and recovered execution state.
- Forward/backward pages retain oldest-to-newest ordering and a captured exclusive
  upper bound. Opaque cursors bind Session, Host incarnation and direction.
  Retained reads slice the requested page; cold reads validate the whole file
  before projecting the page. Queries do not accept new activity or start saves.
- Session creation/activity times and basic model/reasoning/workspace fields are
  available in details/list summaries. Activity is the maximum actual captured
  instant. RFC3339 input normalizes to UTC and uses chronological comparison;
  unrepresentable UTC dates are rejected rather than panicking or backfilling.
- Object schema 2 and history schema 3 require the new metadata. Earlier versions
  are explicitly unsupported. Native Runtime value formats remain unchanged.
  The validator rejects counters contradicting accepted complete/partial model
  responses or tool result batches, while preserving real zero-attempt local
  request failures. Creation time is immutable and saved activity cannot regress.

## Owner boundaries and review fixes

Input acceptance, history acceptance, load recovery and compaction acceptance
register activity work under the same owner lock before publication. They reuse
the existing metadata candidate/baseline/save/reconcile path. Successful saves
coalesce newer accepted activity under that lock before exposing quiescence;
failed candidates remain retained for explicit continuation. No token callback
adds a save, and closing/releasing/reclaimed owners cannot register fresh work
past their final fence.

Automatic compaction appends any already-bound unsaved input prefix before its
marker in the same original append batch. Positions and timestamps already shown
in MessageBound/history remain unchanged. Marker coverage/source position and
active save cursors advance once. Failed append reconciliation confirms that same
batch and exposes one marker without reappending input or rerunning the utility.

Independent source review by the patch worker and Responses worker closed the
compaction ordering, recovered-activity completion fence, message inspection,
terminal timestamp, terminal counter validation and Session timestamp invariant
findings. Review did not substitute for executed tests.

## Validation evidence

All worker commands in the component checks below used `/home/agent/.cargo/bin`
on PATH and at most `-j 2`. The later root workspace gate used four build jobs.

- `cargo fmt --all --check` and `git diff --check`: passed.
- `cargo test -p minicore-host-core --lib automatic_compaction_preserves_bound_input_positions_through_save_and_reconcile -j 2 --locked`: passed. It executes actual first/second Turns with a named scripted model, steering during compaction, successful append and FullWrite failure followed by explicit continuation; compares events, retained/cold history and Turn results with original envelopes.
- `cargo test -p minicore-host-core -p minicore-host-storage -p minicore-host-protocol --all-targets --no-fail-fast -j 2 --locked`: protocol/storage targets passed, including 55 storage integration cases. Core integration fallout identified old zero-counter fixtures, terminal-page expectations, and missing waits for the new real metadata save boundary; these were corrected in this worker or root's `cf4bd05` fixture commit. This intermediate command was not a full pass.
- Subsequent focused executions passed core 129 unit cases, scoped observation 13 cases, Session context 13 cases, prompt-history 12 cases, and tools 5 unit cases. The new message inspection assertion passed. Two tools history equality fixtures were then adapted to compare the new stable envelope fields across retained/cold views; their final rerun belongs to root integration.
- After the final under-lock history-activity registration change, the narrow core/storage/protocol library command executed core 128 passing cases and one read-only fixture failure: its baseline snapshot raced the already-accepted paused-input metadata save. The fixture now awaits that original owner before capturing the exact read-only baseline. The direct gated-append activity assertion and second-Turn compaction regression passed in this run.
- Final `cargo test -p minicore-host-core --lib real_host_cold_read_load_and_runtime_preserve_read_only_boundaries -j 2 --locked`: passed (1 case), preserving the exact before/after read-only assertion after its original metadata owner settles.

The gated append regression first settles the paused-input checkpoint, then gates
the actual history backend before append/settlement. It asserts that the newer
accepted response already owns pending or confirmed metadata and confirms that
metadata while the history file remains unwritten. Other focused regressions
cover coalescing an older in-flight metadata candidate, force-cleared paused
input, durable-history timestamp recovery during shutdown/preparation failure,
fixed backward windows with later appends, UTF-8 and duplicate MessageIds,
Memory result/partial/terminal identity, and missing/invalid timestamp/counter
rejection without file rewriting.

The final protocol timestamp/direction filter passed both cases. Root's
integrated full workspace gate subsequently passed all 1,178 unique Rust cases,
seven doctests and 28 Python checks, plus formatting and strict Clippy/rustdoc.
[Validation evidence](validation.json) records its exact published source,
package checks and native CI status. These root gates include the corrected
history projections and owner-wait fixtures. This worker claims no live provider
or TUI validation.
