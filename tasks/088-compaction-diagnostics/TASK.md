# Compaction diagnostic boundaries

## Scope

Base `d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06`; isolated branch
`task/088-compaction-diagnostics` and worktree `minicore-host-task088`.
Requested worker configuration: Astra/max; effective runtime settings are not
independently attested. Parent owns PROJECT.md, independent review and composed
gates. No push, remote or main action.

Implement only SPEC v4.93 §12 diagnostics at actual compaction boundaries.
No compaction workflow, acceptance, retry, recovery, save confirmation,
projection, dependency, TUI or lifecycle behavior change. The parent subsequently
authorized a separate original-Fault handoff correction after the real red below.
SPEC SHA-256 remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Runtime remains `69faf9196991524b5549499d6208a1eaf567bf56` (0.10.0).

Own `session/compaction.rs`, `session/diagnostics/compaction.rs`, new integration
test `diagnostic_compaction.rs`, and this record. TASK053 owns applying the
exact shared `session/diagnostics.rs` export and `history.rs::settle_save` seam.
TASK053 also owns the later narrowly authorized Session.rs handoff patch.
Query/backend paths remain outside this task.

## Design

Extract short scalar records in the existing owner boundary, emit outside all
access/state guards. Manual association keeps original operation_id and receipt
index. Automatic association keeps its actual Turn, Load, Loop, request and
public response index. Failed calculation and save failure remain independent.
Controlled cancellation is INFO; direct utility failure is one ERROR with its
original Fault identity, never its raw message. History failure remains solely
owned by history.rs. Original-save continuation does not repeat utility work or
completion diagnostics. Logging stores no Span in work and no bodies/resources.

## Evidence and environment

Use `/workspace/shared/recovery-second/task088/env.sh`: Rust 1.90, two jobs,
no incremental compilation, and a fresh exclusive task088 target directory.
All dependency commands use `--locked --offline`.

The new real-provider/file integration test fails on the unchanged production
base with `expected one session.compaction_started`, actual 0, after successfully
running no-subscriber, original-save failure/resume, manual cancellation/failure
and automatic utility scenarios (`red.log`). This is a genuine missing-boundary
red. The initial fixture-only compile mistakes (wrong Turn result identity
field, unsupported Current import, missing CompactionSource import, extra borrow)
were corrected first; `fixture-compile.log` is not production-failure evidence.

## Implementation checkpoint and newly exposed propagation gap

`7527f99fa700f9f1f0b1b19f051c9dc917483792` is the original missing-boundary
red integration test and task record. The ordinary diagnostic change adds
started, cancellation-claimed, committing, finished and DEBUG actual utility
request/usage facts. Its own stored addition is only a monotonic Instant, not a
Span, retained body, logger, dedup flag or event queue.

TASK053 applied the two shared seams in this worktree and authorized including
them with the ordinary diagnostic change. The reproducible shared patch is
`/workspace/shared/recovery-second/task088/applied-shared-seams.patch`, SHA-256
`015d21c9f5a9efefaa6bda0c825925942cf6ffd8317a3c243ba7ee11bae60545`.
The original hand-written patch had an insufficient-context hunk and was not
applied; this was a patch-format issue, not a production conflict.

A subsequent real automatic-HTTP401 probe exposes existing primary-Fault
projection loss: utility failure is SESSION_COMPACTION_FAILED (3702), retained
by compaction; SessionPrompt maps its completion Err to InvalidHistory; Runtime
returns Prompt failure; the existing Host map creates TURN_INTERNAL_UNEXPECTED
(4999). New compaction diagnostics plus existing Turn diagnostics then log two
ERRORs for one failure. See `automatic-forwarding-red.log` (expected one, actual
two). This is not solved by hiding a log. Parent and TASK053 are coordinating a
separate minimal exact-bound business-Fault handoff; its code and regression
will be a separate fix commit, with the original diagnostic change kept reviewable.

Validation paused for the parent's shared-machine performance quiet window.
TASK088 left no Cargo/dependency/compiler process running during it, then
resumed only after the parent released the window.


## Separate original-Fault handoff fix

Ordinary diagnostic checkpoint:
`9887e24745ac2e9ba90c64a106f64a5b1bdc2d6d`.
Before changing the business handoff, `original-fault-red.log` reruns the real
HTTP automatic utility case against exactly that production source and asserts
public `turn.result` should retain SESSION_COMPACTION_FAILED. It fails with
actual TURN_INTERNAL_UNEXPECTED. This stronger red is separate from the prior
duplicate-ERROR observation; the fix does not merely hide that observation.

The parent authorized a narrow ActiveTurn-owned `CompactionFailure` slot with
its original Automatic binding, original Fault and explicit history origin.
Only a settled Failed utility outcome after existing cancellation precedence,
or an actual original-save failure, registers it. No manual operation enters
that slot. An already claimed outcome is not replaced; a later utility failure
is cleanup only, while save faults remain independently owned by persistence.
`claim_report` consumes the slot once, matching Turn, Load, active Loop, returned
Loop and request index. Pinned Runtime increments `requests` only after prompt
preparation, so a failed prompt's original request_index equals report.requests.
A matching Prompt wrapper now yields the original utility Fault, not 4999;
a save-caused wrapper yields no synthetic primary execution Fault. A matching
cancelled report keeps cancellation and the utility Fault only as cleanup.
Other reports retain their existing mapping. Full Fault values are moved,
including any existing diagnostic_id, with no replacement correlation created.
The real HTTP provider case has no invented diagnostic_id and asserts equality
to the original compaction Fault, including its optional association.

TASK053 implemented only the shared Session.rs field, initializer, claim_report
and three callsites. Its exact patch is `original-fault-shared.patch`, SHA-256
`0a91479fc904f6711b927b18216f711b68d269a6822c4d8f96cb38e0dfbeecae`.
The transient returned forwarding fact prevents a second primary ERROR only
after the original business Fault has been restored. No dedup state is kept.

The parent also assigned narrow additions in session/tests/compaction.rs.
Its existing real append gate holds Runtime's terminal history handoff after
an actual invalid utility response. Tests cover retained primary versus
cancellation-first, each with a separate original-inode save refusal where
applicable, explicit original-save resume, exact one-shot cleanup, unchanged
utility call count and no compaction marker for failed utility. Identity-only
probes mutate one binding scalar at a time against the real held owner/report,
including manual, wrong Turn/Load/Loop/request and a different returned Loop.
They are identified as unit probes, not another live integration result.

The public HTTP/file test additionally verifies manual and automatic starts,
manual cancellation, utility failure, confirmed completion, no completion while
an original save is unconfirmed, same-candidate resume, original manual receipt,
automatic Turn/Load/Loop/request/response association, one direct owner ERROR,
failed execution versus persisted save, no and closed subscriber behavior,
reentrant event/new_span/enter/exit/try_close callbacks and TRACE-level payload
exclusion. The script provider is a loopback fixture, not a live external model.
The persistent config fixture initially waited for !preparing instead of the
actual committed enabled config, causing a test-only timeout. It now waits for
that real config fact. See fixture-persistent-config-wait.log; this is not a
production failure, changed timeout or acceptance change.

## Validation

All commands run with the recorded exclusive task088 environment and
`--locked --offline` for dependency operations.

- New real-provider diagnostic_compaction: pass (green.log).
- New gated original-Fault/cancel/save test: pass (green-original-cancel.log).
- `cargo test -p minicore-host-core --lib session::tests::compaction`: all
  16 cases pass (core-compaction-regressions.log).
- Transport compaction, compaction_wait, compaction_wire and
  diagnostic_terminal_failures: all 10 cases pass (transport-regressions.log).
- `cargo clippy -p minicore-host-core -p minicore-host-transport --all-targets
  -- -D warnings`: pass (clippy.log).
- `cargo fmt --all -- --check`: pass (fmt.log); the initial shared seam's import
  order and tuple layout were mechanically formatted, with no semantic edit.
- `cargo test -p minicore-host-core -p minicore-host-transport --doc`: pass,
  zero core examples and one transport compile example (doctest.log).
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core
  -p minicore-host-transport --no-deps`: pass (doc.log).
- `cargo test --workspace --all-targets --locked --offline -- --list` completes:
  919 unique cases across 118 nonempty test executables, 919 listed rows
  (workspace-inventory.log and inventory-summary.json). This is an inventory,
  not a workspace execution pass; nested subprocess output is not counted.
- The Cargo-resolved diagnostic_compaction executable passes 10/10 complete
  repeats (test-artifacts.jsonl, diagnostic-binary.txt, repeat-diagnostic-10.log).
- Final affected strict Clippy and formatting checks pass again after all
  source and shared-seam formatting changes (clippy-final.log, fmt-final.log).
- check-whitespace.sh and git diff --check pass (whitespace-final.log).
- Filtered Linux offline metadata confirms Runtime 0.10.0 at the original
  69faf9196991524b5549499d6208a1eaf567bf56, with no manifest/lock/TUI/SPEC edits.
  SPEC SHA-256 remains the required f044519f…; Cargo.lock SHA-256 is
  1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125
  (metadata-linux.json and protected-files.json).

Original-Fault repair commit:
`a07fd1d60c17cda7d5a56b7083e4d6e6259e6d16`.
The later task-record commit changes documentation only. Worktree is clean
at delivery; no push or other remote action was taken. Parent owns integration,
PROJECT.md, the final composed all-tests gate and independent review. This task
does not claim live-provider, performance or TUI acceptance, nor verification
of the parent's subsequently updated saved-result query API or Runtime pin.
The query fixture Option/async changes at integration are mechanical; preserve
the exact original-Fault slot/claim logic while combining shared Session files.
