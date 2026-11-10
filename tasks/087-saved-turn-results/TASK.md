# Task 087: Read saved Turn results without restoring execution

## Scope and baseline

- Fixed base: `d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06` (tree `34266ef4a0566e3695fe3e3eaa0aad92ef8fb7d2`).
- Isolated branch/worktree: `task/087-saved-turn-results`.
- SPEC v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`, unchanged.
- Runtime: fixed `69faf9196991524b5549499d6208a1eaf567bf56` / 0.10.0, lock/pins unchanged.
- Worker requested configuration: `gpt-6-astra`, `max`; this records the request, not verified effective runtime settings.
- TUI unchanged. No remote actions, push, main integration, or separate result database.

SPEC requires `turn.result` to read the result actually held **or saved** by its Session. Existing execution caches correctly remain empty after restart, but saved `TurnFinished` records were not projected into this query.

## Design constraints

Use existing original TurnId, Input/Runtime/Partial records and public response indexes, plus the saved terminal outcome/reason. The log does not retain LoadId, original config/revisions/counters, or full Fault objects. Do not manufacture them, restore executions, populate completed/output caches, infer outcomes from unfinished records, or associate unbound Summary/Compaction records with a Turn. Existing retained current-run data takes precedence over older saved data. Content reading is asynchronous, cancellation/revocation aware, and outside owner/admission locks.

## Verification

Pending implementation and exact final results. The initial regression is the independent task086 real Host/Runtime, production provider, loopback HTTP/SSE, and actual JSONL probe, copied without weakening its assertion. It is a scripted local HTTP fixture, not a live provider or TUI test.

### Old-source red

On the exact fixed base, the real restart regression failed as expected: the original Turn was completed/persisted with 3 valid JSONL records, cold history was readable, and both cold and explicitly loaded `turn.result` returned `SESSION_TURN_NOT_FOUND` (3201). No active Turn or file mutation was observed. Command: `cargo test -p minicore-host-core --locked --offline --test task087_saved_result_restart -- --nocapture`. Exit 101; 0 passed / 1 failed. Evidence: `/workspace/shared/task087-evidence/old-source-red.log` (worker-local).

All task087 Cargo commands use Rust 1.90, `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, exclusive `CARGO_TARGET_DIR=/workspace/shared/minicore-host-task087-target`, `CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, and `TMPDIR=/workspace/shared/minicore-host-tmp`.

## Implementation checkpoint

- `TurnResultData.info` and `.config` are deliberately optional retained metadata. Current-run JSON keeps its original `info`/`config` objects; saved-only results omit them and `load_id`, and carry `saved_terminal: { outcome, reason }` from the actual `TurnFinished` record. No original Fault, revisions, attempts, configuration, or load identity is fabricated. The saved reason is not a structured Fault.
- `turn_result_snapshot` is async. `turn.wait` returns an already-saved terminal directly, without restoring a waiter/execution. Current active/finalizing Turns still fail a premature result query and cannot be upgraded by readable unconfirmed bytes.
- `HistoryBackend::read_saved_turn` is an explicit narrow read capability. Its default is an unsupported-read Fault, not a fabricated empty result; memory missing results never invoke it. The production backend uses `SessionStore::read_history` cold, with whole-file validation before filtering, or the existing journal's validated opening records when loaded. The latter is not used as a current-run history cache. Only matching original Turn records are cloned.
- Current completed/active ownership, caller validity, and source visibility are rechecked after I/O and before returning the projected result. No owner/access lock crosses I/O. Retirement remains serialized by the existing disk-only guard. Reads never load, append, reconcile, create caches, or rewrite JSONL.
- Existing assertions remain; test edits outside dedicated regressions are mechanical optional-metadata access and async adaptation. The only `session.rs` change is the narrow trait method. No manifests, exports, dependency locks, TUI, README or PROJECT edits.

### Focused evidence before the affected gate

- `cargo test -p minicore-host-core --locked --offline --test task087_saved_result_restart -- --nocapture`: **4 passed**. Actual Host/Runtime and production provider over loopback HTTP/SSE, real complete+tool result, failed and cancelled partials, normal shutdown/reopen, cold/loaded reads, new Turn after load, persistent close, exact saved reason, byte/state stability, and no extra resolver/model/tool starts. Missing result, valid unfinished history, corrupt trailing record, and absent history file stay distinct.
- `cargo test -p minicore-host-core --locked --offline --lib session::observation::query`: **7 passed**, including 5 new deterministic journal/owner tests and 2 preserved tests. Covers loaded journal before owner publication, read revocation without cancelling a concurrent load, real resource-retirement fence, body reclamation, and public response indexes across two actual Runtime Loops. Explicit storage seeds/gates are labeled fixtures, not claimed live execution data.
- The adapter regression uses actual typed operations, OS pipes, authenticated loopback WSS and Runtime. Only its model event source and test TLS/access credentials are fixtures. The saved DTO/identity and `turn.result`/`turn.wait` match on cold and loaded paths.
- New fixture defects discovered and corrected while constructing tests: retirement sets its internal fence before publishing a state notification, so the deterministic test now polls that actual fence; the scripted complete empty model response requires a valid ProviderReplay marker, as in the existing multi-Loop fixture. These were new-test defects, not weakened assertions or production workarounds.

Full affected tests/lint/docs/list are pending at this checkpoint. Root owns composed workspace gate, independent review and publication.

## Verified affected gate

Implementation fixed commit: `98d476f045e1f775a56696339a65451a7b1f10ed`, tree `547bed720aa62be36978fef584742e2d9b944d3e`. The only earlier task commit is `71d393f` (old-source red). All commands below completed successfully against that code in the task-exclusive build directory:

1. `cargo fmt --all -- --check`, `./scripts/check-whitespace.sh`, `git diff --check`.
2. `cargo clippy -p minicore-host-core -p minicore-host-transport --all-targets --locked --offline -- -D warnings`.
3. `cargo test -p minicore-host-core -p minicore-host-transport --all-targets --locked --offline`: **480 unique Rust tests passed**, with no failures. The new saved-result adapter test runs exactly once, without importing a second copy of an unrelated pipe-fixture test.
4. `cargo test -p minicore-host-core -p minicore-host-transport --doc --locked --offline`: **1 doc test passed** (core has 0).
5. `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core -p minicore-host-transport --no-deps --locked --offline`.
6. `PYTHONDONTWRITEBYTECODE=1 python3 scripts/task059_projection_test.py` and `PYTHONDONTWRITEBYTECODE=1 python3 scripts/task058_rss_test.py`: **15 + 5 Python tests passed**.
7. `cargo test -p minicore-host-core -p minicore-host-transport --all-targets --locked --offline -- --list`: **480 unique (test binary, test name) cases**, matching the affected run.

Worker-local evidence and SHA-256:

- `/workspace/shared/task087-evidence/old-source-red.log`: `d0be3978b1fee2fdd98fc1ee73b3c3d68e94665947c8fbbe5d68ef82b3c6030d`.
- `/workspace/shared/task087-evidence/restart-regression.log`: `b51db18c7c4d4026dfeb462cc04f525cc5f9de2a6c5434004931274b61b28717`.
- `/workspace/shared/task087-evidence/query-unit.log`: `12996ff00c399d90b8985047aaa400df845047ad490e425f64935e3b105e8a36`.
- `/workspace/shared/task087-evidence/affected-gate.log`: `6978a027a65b21478c9c7db4bf918a44afb3dbd123d3cf17caef7828b3d50791`.
- `/workspace/shared/task087-evidence/affected-test-list.log`: `aa6dfda8909055fe60dafeb81a61e5eab0001792c63f072379473b6dfd71ff05`.

## Remaining boundaries

- This is an **affected-package** gate, not the root's composed full-workspace run or independent review. Root owns integration with concurrent diagnostics changes, full workspace `--list`/gate, and publication. No push, remote or main action was performed here.
- The existing raw native owner/collaboration execution APIs remain retained-execution APIs; public `turn.result`/`turn.wait` routing and the typed/stdio/WSS query DTO use the saved-fact projection. `turn.get`, `turn.list`, tool/output APIs and execution caches were not expanded into historical execution recovery.
- A loaded journal's opening baseline is an existing accepted saved source, not a live disk cache. Current results are always read from their actual owner. Missing original LoadId/config/revisions/counters/full Fault stay absent. No historical execution is resumed or fabricated.
- A read cannot restore or infer a terminal result from a valid unfinished log; it returns `SESSION_TURN_NOT_FOUND`. Whole-file storage corruption/missing-file failures remain the original load Faults. Existing pending save state is never acknowledged or repaired by reading.
- SPEC hash, Runtime revision, lock file, TUI, and stored record schema remain unchanged. No live provider/TUI validation is claimed.

## Integration and result-branch validation

The three task commits are integrated as 158f0f7, 163e904 and b31f73f,
respectively, on the coordinator development branch. A later Session control
fixture requires only optional retained-info access; its original cancellation
assertion is unchanged.

Independent review at source6032f8f confirmed a P2 wire-contract defect in the
first candidate: three independently optional metadata fields admitted nine
invalid metadata/source combinations, including neither branch and both branches.
The real stdio client accepted all18 malformed turn.result/turn.wait replies;
a manually built invalid public struct also serialized successfully. This is
not evidence that the real owner emitted such values. The independent red log
has SHA-256 bea2b533f6bca1e398abe8ce2fa5646645383cdf8301bcf2f6ed1c73f8ff67d8.

The coordinator's narrow correction validates both serialization and decoding:
retained results require info+config and an actual LoadId; saved-only results
require saved_terminal and omit LoadId. The two valid JSON shapes and public
fields remain unchanged. Serialization borrows bodies instead of cloning them;
unknown structured Fault leaves remain handled by their existing codec. No
historical execution metadata is synthesized and no storage/owner behavior changes.
Four new wire tests enumerate metadata branches, LoadId association, round trips
and invalid public-struct serialization. Composed query tests and actual
control/Closed plus typed/pipe/WSS saved-result tests passed before commit;
fixed-commit strict checks and independent green replay follow separately.
