# Task 119: Original startup cards for independent agent execution

## Authorization and checkpoint

The owner approved a continuously updated agent card anchored to the original
startup ToolCall, native child conversation previews, subsequent-send references,
truthful completion, retained-history reopening, and a 0.0.1 patch increment.
The requested implementation worker configuration is `gpt-6-astra` with `max`
reasoning; this records the request, not independent runtime attestation.
The Host worker owns this repository; the TUI worker owns its separate repository.
The coordinator owns publication and exact release commit mapping.

- Remote Host baseline: `36eac8bf322a8276c7aca0e310814d615216d8de`, version 0.12.5.
- Verified baseline tree: `acdabacad4f60d338050ba23b5e57f15db3173a2`.
- Local materialization checkpoint: `d13bedac`; it is not the remote commit.
- Target: Host 0.12.6 / TUI 0.12.7, SPEC 4.11.5.
- SPEC SHA-256: `4459dcbf395552e7e6bdab94ec322a93391d8a5a7d0721f1947afbfa7b929e2c`.
- SPEC and AGENTS were updated before production code.

Status: the original feature checkpoints were published, but delivery is not
complete. Host `95cc1dd4a42d25372be7c3402c6cc4611f98fd15` passed all nine jobs
in [CI run 44108519829](https://github.com/zqcli/minicore-host/actions/runs/44108519829).
TUI `60c3a0f3420a24d8167df8114fc49a9f652fe3d5`, pinned to that Host revision,
failed compilation in [CI run 44110508243](https://github.com/zqcli/minicore-tui/actions/runs/44110508243).
Its tests, Rustdoc and dependency checks were not reached. Both versions remain
the planned single patch increment; neither feature was in `dev` at takeover.

## November 18 continuation

The existing feature history is preserved in isolated checkouts. The original
ChatGPT generation status could not be inspected; the published CI runs were
finished, not queued or running. No duplicate implementation was started.

Independent reviews identified these necessary corrections:

- A temporary intermediate parent can be reclaimed while its delegated child
  continues. Valid same-scope sends must not depend on looking up that original
  parent. The child relation's existing weak parent permits optional original
  card projection; the current sender records facts from the existing delegation
  permit's child owner or fixed terminal association, including fast release.
- TUI stable Rust compilation uses `Arc<str>::as_ref()` and an owned sanitized
  label. Saved ToolResult delegation metadata is installed on the actual later
  send as well as the original card, including late canonical history metadata.
- Native baselines and late output cannot resurrect clipped old model responses
  as the latest/final answer. Completed tool state does not recreate unpositioned
  old preview blocks; finished baseline Turns release their transient buffers.
- Large native bash/read results keep decoded newlines and actual recent output.
  Generic JSON formatting retains its true bounded tail. Retained preview bodies
  and formatting buffers are bounded; native JSON decoding can still use memory
  proportional to the original result. No new decoder framework is introduced.

Regression additions use real Host/Runtime ownership with explicit gated models
for nested memory-parent reclamation, fast-child projection after release and
40-round child baseline recovery. History-only send references, native result
tails, Unicode clipping, rendered row budgets, internal blank lines and copy
chrome have focused tests. The four real-Host frontend card tests pass after
correcting their original fixture: native model inputs include provenance headers,
so parent/child calls are identified by loop identity and decoded input provenance,
not by comparing the whole native message with a bare task string.

### Continuation verification

The owner approved installing the official Rust toolchains. On Linux with the
repository-pinned Rust 1.90.0, the complete `scripts/check.sh` exits 0:

- Formatting and workspace/all-target strict Clippy pass.
- All 1,263 independently enumerated Rust test cases pass, with no ignored cases.
  The count comes from `cargo test --workspace --all-targets --locked -- --list`,
  not from summing nested subprocess test output.
- All seven doctests, strict workspace Rustdoc and 28 Python checks pass.
- The nested reclaimed-parent, fast released-child and delayed acceptance
  regressions are included in that full run.
- All 225 external lock records, the planned 0.12.6 package versions and the
  SPEC hash above are unchanged.

The first attempt found a test-only Clippy clone warning, then an environment
failure because the default HOME was read-only. The equivalent slice assertion
was corrected and the complete gate rerun with an isolated writable HOME. No
configuration production behavior was changed to hide that environment limit.

The repaired TUI uses current stable Rust and needs its complete exact-Host-pin
quality and release-build checks separately. New cross-platform CI, a physical
terminal session, a live external model and performance acceptance are not
claimed by this local Host gate. No new push, merge, tag or release was performed.

## Owner changes

Serializable delegation facts have one protocol definition, re-exported by core
and reused by storage. An association identifies the original parent Session,
ToolCall, child Session and first task MessageId. It includes actually accepted
input identities, the real shared Turn ranges, and optional actual settlement
outcomes and discarded inputs. A bare ToolCallId does not identify a card.

The child Session projects current bindings through existing session state and
queries. Parent ToolCall state records acceptance and actual handoff with an
advanced state revision. A delayed acceptance cannot replace a terminal fact.
The child fixes immutable terminal metadata at its real handoff boundary; final
notification metadata comes from that fact, independently of the parent view.
The existing tool return and child execution lifecycles remain separate.

Agent operations returning a business Fault now use the existing native failed
tool outcome with the unchanged `{error: Fault}` body. The model still receives
the ordinary tool result and can continue its Turn. Partial creation receipts
keep their existing typed steps and already accepted child work is unchanged.

Native history records retain small typed associations beside unmodified Runtime
values. The original ToolResult saves startup facts; the later consumed final
notification saves its settled association. History and Turn-result reads expose
the same recorded facts. Neither restart nor a missing child source recreates execution,
queued input, a delegation or child content. Temporary memory children still
close before the parent's final notification is delivered.

The client SDK adds explicit `OutputBuffer::retain_tail`, `start` and `end`.
Discarding an already observed prefix preserves absolute source positions,
UTF-8 boundaries, overlap validation and missing-tail truth. This limits the
contiguous preview text only. Disjoint pending gap ranges are not bounded by
this method; normal full-transcript retention remains unchanged.

## Original worker validation (before the published feature CI)

All local Rust commands source `/workspace/scratch/ea58f4af9322/rust-env.sh` and
use the pinned graph with `--locked --offline`.

- `cargo check -p minicore-host-protocol -p minicore-host-core --lib --locked --offline`: passed.
- `cargo check -p minicore-host-protocol -p minicore-host-core -p minicore-host-tools --all-targets --locked --offline`: passed.
- `cargo test -p minicore-host-core --lib settled_delegation_survives_delayed_acceptance_and_saves_typed_notice --locked --offline`: passed, one actual-owner gated race.
- `cargo test -p minicore-host-tools --test delegation_acceptance_fences --locked --offline`: passed, six cases including the native business-Fault outcome.
- `cargo clippy -p minicore-host-protocol -p minicore-host-core -p minicore-host-tools -p minicore-host-storage --all-targets --locked --offline -- -D warnings`: passed.
- The complete protocol/core/storage suites passed in the four-package all-target
  run. That command then stopped at a tools exact-shape expectation that did not
  yet include the newly recorded notification metadata. Its full existing body
  assertions were preserved and the expected real association was added.
- `cargo test -p minicore-host-tools --test task070_inspect_recovery actual_child_input_and_completion_notification_keep_sources_after_restart --locked --offline`: passed after that expectation update.
- `cargo test -p minicore-host-tools --all-targets --locked --offline`: passed,
  including live and cold history/result projections and all owner-tool cases.
- `cargo fmt --all`, `git diff --check`, and all 28 Python checks from the three
  scripts in `scripts/check.sh`: passed. All 225 external lock records are
  unchanged; all 14 workspace packages are 0.12.6.

An earlier broad run observed a missing `host.started` diagnostic in
`lifecycle_publication_and_runtime_keep_actual_identities_and_phases` (expected
one row, received zero). The same case passed in an isolated rerun and in the
later complete 22-case diagnostic binary. No diagnostic production code was
changed and no claim is made that the failure was proven to predate this task.

Focused tests cover a busy child whose M2 steer binds before M1 follow-up,
shared inputs and multiple Turns, failed/cancelled Turns with preserved input,
explicitly removed first input, repeated child Session reuse with a new anchor,
immutable startup results, terminal state revisions, and saved typed notification
facts. Persistent owner integration additionally checks cold history reopening
without restoring a removed child. The genuine transport gap/replay test was
extended for preview clipping, Unicode boundaries, late reads and missing tails;
running that transport test remains part of full CI.

The complete workspace gate is blocked locally by missing pinned registry
archives, first reported as `asn1-rs 0.7.2` for transport certificate test
dependencies. Dependencies are not changed to bypass the environment. Full
formatting, Clippy, Rust tests, doctests, documentation checks, cross-platform CI
and the authoritative unique-case listing remain coordinator release gates.
No live external model, terminal UI or performance acceptance is claimed here.

## Next steps

Keep the passing Host gate and finish the TUI gate against its exact repaired
Host pin, recording unique test counts and validation limits. Publish only within the owner's explicit authorization, then verify
full CI on those exact commits before integration or release completion.
