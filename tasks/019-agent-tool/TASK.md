# Implement the agent tool

## Baseline and scope

- SPEC v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Host baseline: `775235a9e9f7031ba654b6b83ddc73ecf9a5da6a`.
- Worktree: `/workspace/shared/minicore-host-agent-tool`, branch `feat/agent-tool-adapter`.
- Owned paths: `crates/host-tools/**`, this task record. Root manifest/lock changes are isolated in bootstrap commit `8145239` for parent integration.
- Read AGENTS and SPEC chapters 2, 3, 5, 7, and relevant 9. Preserve direct owners, double queues, real result handoff, and independent child lifetime.

## Stage 1

- Implement distinct create/send/inspect/update/control/wait inputs, strict branch fields, and Runtime's actual ToolSpec.
- Reuse protocol config values, independent source snapshots, same-snapshot revisions, array/structured replacement, and explicit null clearing only where supported.
- Define narrow typed owner integration ports. No external CallerContext is cloned or manufactured, no model source/internal field grants authority.
- Seven focused contract tests passed. The initial red test caught serde's unit-variant unknown-field acceptance; close_scope now uses a strict empty struct variant.

## Exact commands and results

Environment: `RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo PATH=/workspace/shared/cargo/bin:$PATH TMPDIR=/workspace/shared/minicore-host-tmp CARGO_TARGET_DIR=/workspace/shared/minicore-host-agent-tool-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0`.

- `cargo check -p minicore-host-tools --offline`: passed.
- `cargo fmt --all`: passed.
- `cargo test -p minicore-host-tools --locked`: 7 tests passed, 0 doc tests.
- Full workspace gates and unique `--list` count: see Stage 2 below.

## Stage 2

- Stage 1 implementation commit: `0ed6b2a`.
- Implement Runtime AgentTool and selected-only ToolRegistry, ordinary business Fault output, and explicit wait facts with Runtime cancellation/deadline support.
- Implement adapter-owned root/member relationships, registration before task acceptance, same-scope append, cross-scope delegation rejection, actual input-range qualification, one final notification, temporary cleanup, and scope seal/close without awaiting the current root ToolCall.
- Separate current-Turn dispatch from independent notification permits. Root owner seal and actual root_closed hooks fence lifecycle and registry removal.
- Preserve startup cleanup across a dropped Runtime ToolFuture. Ordinary empty creation is not auto-closed. A delegated memory creation's narrow cleanup permit only closes its actual new Session after task non-acceptance.
- Notifications carry actual outcome and final public text, not full child history or reasoning. They retain original direct parent, original task MessageId and actual ToolCallRef; a failed delivery is recorded once with no retry.
- 26 unique tools tests pass: 7 contracts, 9 adapter, 9 scope, 1 actual Runtime AgentLoop test. All owner ports and model replies in this stage are explicit fixtures.
- `./scripts/check.sh`: full workspace whitespace, fmt, clippy with `-D warnings`, all-target tests, doctests and rustdoc with `-D warnings` passed against this stage baseline.
- `cargo test --workspace --all-targets --locked -- --list`: 325 listed test cases. Nested storage subprocess output is not added to that count.
- Gate log: `/workspace/shared/minicore-host-agent-tool-stage2-check.log`; unique list: `/workspace/shared/minicore-host-agent-tool-stage2-list.log`.

## Remaining integration

- Finish the final published Runtime retry pin, full integrated workspace gates,
  unique test count and independent review. No remote push is authorized here.
- Production owner integration is now implemented and verified below; fixture
  coverage remains identified separately and no live provider/TUI claim is made.

## Owner-interface refinements

- Preserve name/title from the same atomic config snapshot; submit complete metadata and configuration with the original revision.
- The completion contract separately carries actual consumed input ranges and explicitly discarded inputs. A task discarded before consumption reports `not_executed`, with no invented TurnResult or success outcome.
- Scope cleanup retains failures as actual blockers, caches only success, and permits a later explicit close to continue original owner cleanup. The first close does not automatically retry it.
- Runtime 0.10 focused gate: `cargo test -p minicore-host-tools --locked` has 29 passing tests; `cargo clippy -p minicore-host-tools --all-targets --locked -- -D warnings` and formatting pass.
- A production `core_adapter.rs` and actual Host/Runtime integration tests are being prepared against the sole core owner's capability API. They are not included in the verified build until the corresponding integrated core and observation commits are available.

## Stage 3: production owner bridge

- Merged the parent's isolated observation/delegation integration at `46c90e8`
  (`d06fde3` locally), then metadata `5e41795` (`722aabc`), paged-history
  capability `ec31cda` (`6cf223c`) and preparation handoff fix `04fc9b8`
  (`8d87a4a`). No existing owner/transport files were edited by this task.
- `CoreTurnPort` now uses core-minted active Turn capabilities and independent
  creation, delegation and root cleanup permits. Scope hooks are weak back
  references to avoid a scope/permit/hook reference cycle.
- Inspect maps all six target kinds to actual owner reads. Agent summaries use
  real Session states. Session history uses the authoritative pager; Turn and
  ToolCall text reuse `TextReadRequest`. The strict model schema and runtime
  parser reject invalid page/range combinations and null optional read inputs.
- Public body projection removes provider replay and Loop metadata. Structured
  ToolResults remain structured, and a selected text range does not also return
  the entire tool output. Notifications distinguish discarded tasks from a
  real bound-but-cancelled Turn with zero model requests.
- Four production Host/Runtime tests now pass: all six actions in one Turn;
  child handoff/notification after parent completion and client disconnect;
  missing real factory leaves the message queued with no candidate Turn
  publication; root `close_scope` closes and reclaims actual owners without
  joining its own ToolCall.
- The scope-close regression initially exposed a core preparation-exit gap:
  an accepted task discarded while preparation settled could leave the child
  Closing with no result handoff. The sole core owner fixed this in `04fc9b8`;
  the same test now completes actual close/join/reclaim without loosening the
  lifecycle assertion or extending the test deadline. The current Turn may
  reach a model request boundary before the independent close cancellation.
- Temporary verification only: `cargo test -p minicore-host-tools --offline
  --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-retry-state"'`:
  **35/35** pass (4 real Host/Runtime, 1 Runtime with port fixtures, 30 contract
  and owner-port fixture tests), with Runtime source fixed at `3cf57e3e1947b0464bd8843ed0508bfddfb2639a`.
- Focused Clippy `--all-targets -- -D warnings`, format and whitespace checks
  pass. This temporary patch is not a deliverable dependency and is not
  committed. Final no-patch integration gates remain required.
- Logs: `/workspace/shared/minicore-host-agent-tool-core-integration.log`,
  `/workspace/shared/minicore-host-agent-tool-close-scope-red.log`.
