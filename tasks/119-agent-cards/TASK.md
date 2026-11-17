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

Status: Host implementation and independent owner review are complete; the
available-package local checks described below pass. No commit or publication
has been performed by the Host worker. The coordinator owns feature CI and the
final release checkpoint; their exact identities and results must be recorded
before claiming release completion.

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

## Validation

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

Publish an ordinary feature checkpoint for full CI, then let the coordinator
record the exact released Host revision and corresponding TUI pin.
