# Independent compaction diagnostics and Fault handoff review

## Verdict and fixed source

**APPROVE, scoped to `a07fd1d60c17cda7d5a56b7083e4d6e6259e6d16`.** No blocking
finding remains in the reviewed original-Fault handoff or compaction diagnostic
change. This is not approval of a later combined tree, the complete Host product,
live external-provider behavior, or TUI integration.

- Base: `d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06`.
- Original diagnostic red: `7527f99fa700f9f1f0b1b19f051c9dc917483792`.
- Diagnostic checkpoint: `9887e24745ac2e9ba90c64a106f64a5b1bdc2d6d`.
- Original-Fault fix: `a07fd1d60c17cda7d5a56b7083e4d6e6259e6d16`.
- Independent worktree: `/workspace/shared/minicore-host-task092-review`.
- Exclusive build directory: `/workspace/shared/minicore-host-task092-target`.
- Private evidence: `/workspace/shared/minicore-host-task092-evidence`.
- Requested worker configuration: `gpt-6-astra` with `max` reasoning. No independent
  effective-runtime attestation interface is available; those settings are not
  claimed as independently verified.

Initial static inspection and a compile-only warm-up used exactly `9887e24`.
The author's uncommitted diff was read only to understand the proposed change.
All positive behavioral results below use fixed `a07fd1d`, not that mutable diff.
Production files were not edited. The only repository addition is this report;
PROJECT.md and final integration remain with the parent. No push, main update,
remote mutation, dependency change or SPEC change was performed.

SPEC v4.93 SHA-256 is unchanged:
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Cargo.lock SHA-256:
`1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`.
This source pins Runtime `69faf9196991524b5549499d6208a1eaf567bf56` / 0.10.0.
Agent `f36a87366d8e2fe0a1c8216463887902e471c739` is reference material, not a
behavior baseline. TUI reference `3beeb1d8a525e58ce6352e05061f242c5580e248`
remains unchanged and its Runtime lock `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`
is not this Host's Runtime pin.

## Reviewed behavior

1. The one private ActiveTurn slot carries the actual Automatic Turn/Load/Loop/
   request binding and complete Fault value, plus utility-versus-history origin.
   It is neither a global cache nor an event-reconstruction or recovery entry.
   Pinned Runtime increments `requests` only after successful prompt preparation;
   the failed prompt's unissued request index therefore equals `report.requests`.
   Both real request zero and an independent real request-one failure pass.
2. An exact utility failure crosses SessionPrompt/Runtime as the original
   SESSION_COMPACTION_FAILED (3702), not TURN_INTERNAL_UNEXPECTED (4999).
   Full Fault equality, including the optional diagnostic identity, is preserved.
   The real HTTP Fault currently has no diagnostic_id; none is invented. A
   separate white-box test covers a populated diagnostic_id.
3. Cancellation that claims the Turn before report handling remains Cancelled.
   Its already-produced utility Fault appears exactly once as cleanup. An
   existing different primary remains primary; consuming/reclaiming the same
   report does not add a second cleanup Fault. Wrong Turn, Load, Loop, request,
   or Manual bindings do not hijack the normal report mapping.
4. An automatic compaction save refusal remains a separate 3301
   persistence_fault. The public Turn has no manufactured 4999 before or after
   original-inode repair. Explicit original-save continuation confirms the same
   candidate once, does not call the utility again, and does not turn failed
   execution into completed execution. A late cancel cannot replace that claim.
5. One direct utility owner records the primary ERROR. The transient forwarded
   return fact suppresses only the duplicate wrapper ERROR after business-Fault
   restoration; no persistent logging/dedup flag drives completion. An independent
   later history failure retains its own ERROR and save ownership.
6. Manual diagnostics retain their original operation and receipt index without
   inventing a Turn. Automatic records retain real Turn/Load/Loop/request and
   public response association. Scalar records are emitted after state/access
   locks are released; no source/candidate body, owner, logger or Span is retained
   by a new diagnostic field. No and closed subscriber paths remain business-
   functional. Existing reentrant subscriber and TRACE privacy assertions pass.

Cancellation precedence before utility failure is unchanged: a cancelled utility
is Cancelled, not an invented failed utility. The populated-ID, wrong-binding and
other-primary unit inputs below are deliberately constructed and are not claimed
as additional naturally occurring production failures.

## Independent probes

`probe.rs` uses real Host/resources, bound Runtime, production HTTP/SSE adapter,
production read tool and an actual append-only file Journal. Only loopback model
responses are scripted. It is compiled against artifacts selected from the fixed
source's Cargo JSON output, not against another worktree's target directory.

- Request zero executes the real read tool. Automatic compaction then fails at
  request index one with real HTTP 401. Public requests remains one, the original
  complete 3702 Fault is retained, and the sole direct ERROR has request_index=1.
- The next explicitly resumed, compaction-disabled Turn completes without the
  old fault or cleanup entry. There is no utility retry or extra dialogue call.
- With the diagnostic writer already closed, an incomplete HTTP/SSE utility
  response still fails through the real adapter/engine as original 3702. Its next
  Turn also remains clean.
- Replacing history.jsonl with another inode causes the real Journal to refuse
  the automatic summary save. The public pending result retains original 3301
  only as persistence_fault and no primary/cleanup Fault. Repairing the inode and
  explicitly resuming preserves Failed + Persisted, clears the save block, writes
  exactly one compaction marker, and keeps the HTTP request count at three.

The final probe passed once after compilation and ten additional fresh-process
repetitions. These are repeated executions of one test, not extra unique tests.

`whitebox.rs` adds one test only to a private copy of fixed host-core source. The
builder byte-compares every original Rust file; its only difference is the test
addition in session/tests/compaction.rs. Production checkout files remain intact.
A real utility-engine failure and real blocked original-inode save first produce
an actual ActiveTurn/report. Deliberate unit inputs then exercise `claim_report`:
exact binding with populated diagnostic_id; each wrong scalar and Manual;
existing independent primary with one cleanup; duplicate claim; and missing slot
retaining existing 4999 mapping. The original state is restored before the real
save is repaired and completed. This unit test passes; it is not a model/API
integration claim for those deliberately altered inputs.

## Commands and outcomes

Environment (`env.sh`): RUSTUP_HOME=/workspace/shared/rustup,
CARGO_HOME=/workspace/shared/cargo, PATH prepends its bin,
TMPDIR=/workspace/shared/minicore-host-tmp, CARGO_BUILD_JOBS=2,
CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0, CARGO_INCREMENTAL=0,
and the exclusive CARGO_TARGET_DIR above. Rust/cargo both report 1.90.0.
All Cargo dependency operations use `--locked --offline`.

```sh
cargo fmt --all -- --check
cargo test -p minicore-host-core --lib compaction --locked --offline
cargo test -p minicore-host-transport --test diagnostic_compaction --test compaction --locked --offline
cargo clippy -p minicore-host-core -p minicore-host-transport --all-targets --locked --offline -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core -p minicore-host-transport --no-deps --locked --offline
./scripts/check-whitespace.sh
cargo test -p minicore-host-transport --test compaction_wait --test compaction_wire --test diagnostic_terminal_failures --locked --offline
cargo test -p minicore-host-core -p minicore-host-transport --doc --locked --offline
cargo test -p minicore-host-core --lib --locked --offline
cargo test -p minicore-host-transport --test diagnostic_compaction --no-run --locked --offline --message-format=json
cargo test -p minicore-host-core --lib --no-run --locked --offline --message-format=json
bash /workspace/shared/minicore-host-task092-evidence/probe-build-run.sh
python3 /workspace/shared/minicore-host-task092-evidence/whitebox-build-run.py
```

All final commands pass. Core compaction filter: 23 tests; later complete core
library: 102 tests, which includes those 23. Targeted transport suites: compaction
3, compaction_wait 5, compaction_wire 1, diagnostic_terminal_failures 1,
diagnostic_compaction 1. Doctests: core 0, transport 1. Two independent private
test cases pass. These are suite results, not a complete workspace inventory or
full workspace pass. Formatting, affected strict Clippy/rustdoc and whitespace
pass. No performance inference is drawn from elapsed test times.

A first status message accidentally attributed the author's ten transport cases
to the reviewer's initial four; it was corrected immediately and the missing
seven were actually run. Only the commands and counts above are claimed.

### Fixture corrections, not production findings

- The first public probe incorrectly required a nonempty diagnostic_id. SPEC
  permits absence; it now asserts exact optional identity equality. The original
  failure log is retained as `probe-fixture-optional-diagnostic.log`.
- The first private white-box compile omitted the sibling test-support directory.
  Its missing-file compile log is retained as `whitebox-fixture-path.log`.
- On repetition, the public probe tried resume after Finalizing but before the
  original driver had returned. The real existing API correctly refused with
  SessionBusy. The fixture now bounds waiting for this specific *unaccepted*
  admission, reuses its operation ID, stops after the first acceptance, and does
  not retry a successful save or model call. Other errors still fail the probe.
  `probe-fixture-early-resume.log` retains that original result. The final ten
  repetitions pass without changing production code, deadlines or assertions.

## Evidence hashes

All paths in the following table are relative to the private evidence directory.
`manifest.sha256` also includes the scripts and executable hashes. Full rustc
link commands and selected artifact paths are retained in the probe logs.

| Artifact | SHA-256 |
| --- | --- |
| `fixed-gate.log` | `db9c114dda504d862d777798e80403fe37228b6326588ef49c6844cbfd9bf49e` |
| `extra-gate.log` | `5782c57169c610b1b8703d1b26b16292fa3c70f0eead7bb4d0a6a4482a3168d2` |
| `full-core.log` | `be34b991b007a03a1745820856793bc9ba135e6fc46826f131edeacf24d1cc57` |
| `probe.rs` | `35296632317bd46ed4873a400ee7909065f228cdbc057fafdb0b1597238a3c47` |
| `probe.log` | `812abe5a236fc116f5f38c6b2bb621baca31621c10c75f9051370da2bef92e8d` |
| `whitebox.rs` | `d47669318c2b060547058d2e4e8a2400538ab5ddf2eb82e8f63cbe17a0516e5b` |
| `whitebox.log` | `684a26c85a0d7460ac48e2a45df3e3d368d5e475cf2da49fb2ea536c17c277f5` |
| `review-scope.txt` | `61a9cafcdfdf87ecafd44971f7d1f45d07d154c31f17c9448187efb71102e3ff` |

## Remaining boundaries

- No full workspace all-target test run, complete workspace inventory, Python
  check suite, live external-model run, performance measurement or TUI run was
  performed for this review. The parent owns the composed final gate.
- Parent query/DTO changes and Runtime updates newer than this fixed source are
  not in this checkout. In particular, the author's tests still use the older
  non-Option SessionInfo.config API. Adaptation and tests of the final combined
  source are mandatory before attributing this approval to that combined tree.
- No exhaustive arbitrary scheduler/interleaving proof is claimed. The review
  uses narrow deterministic real barriers and separately labeled unit inputs.
- The producer's older red logs were inspected/referenced as provenance; they
  are not represented as independently rerun red evidence in this task.
