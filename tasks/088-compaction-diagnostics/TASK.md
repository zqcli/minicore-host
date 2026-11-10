# Compaction diagnostic boundaries

## Scope

Base `d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06`; isolated branch
`task/088-compaction-diagnostics` and worktree `minicore-host-task088`.
Requested worker configuration: Astra/max; effective runtime settings are not
independently attested. Parent owns PROJECT.md, independent review and composed
gates. No push, remote or main action.

Implement only SPEC v4.93 §12 diagnostics at actual compaction boundaries.
No compaction workflow, acceptance, retry, recovery, save confirmation,
projection, dependency, TUI or lifecycle behavior change. SPEC SHA-256 remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Runtime remains `69faf9196991524b5549499d6208a1eaf567bf56` (0.10.0).

Own `session/compaction.rs`, `session/diagnostics/compaction.rs`, new integration
test `diagnostic_compaction.rs`, and this record. TASK053 owns applying the
exact shared `session/diagnostics.rs` export and `history.rs::settle_save` seam.
Do not edit Session.rs or query/backend paths.

## Design

Extract short scalar records in the existing owner boundary, emit outside all
access/state guards. Manual association keeps original operation_id and receipt
index. Automatic association keeps its actual Turn, Load, Loop, request and
public response index. Failed calculation and save failure remain independent.
Controlled cancellation is INFO; direct utility failure is one ERROR with its
original Fault identity, never its raw message. History failure remains solely
owned by history.rs. Original-save continuation does not repeat utility work or
completion diagnostics. Logging stores no Span in work and no bodies/resources.

## Validation in progress

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
