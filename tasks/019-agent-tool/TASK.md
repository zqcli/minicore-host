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

- Core admitted-Turn internal operation capability and registered delegation result handoff are being implemented by the assigned sole owner. The observation output hook runs after claim and must not be used for fallible business preparation.
- Bind selected tool implementations before consuming initial input; preserve the frozen ToolSet across all internal loops.
- Connect the implemented scope/notification/Runtime adapter to the real core capability implementation.
- Add real owner adapter and Host end-to-end tests once core capabilities land. Do not present fixture tests as real Host delegation.
- Incorporate the observation owner's actual output/page request types rather than invent a parallel read protocol.
- Metadata updates require atomic owner name/title plus config revision commits; bridge to core once that owner publishes the implementation.
- Stage 2 code commit: `b9066a5`; its additional root-lock-only test dependency commit is `c8a51f0`.
- Integrated parent baseline `275110695551c30582d1302e6b01ab23a3aad536` with local merge `8ca9ab9`. The tool crate now uses the actual published Runtime `59ee8ce668f72a21d408e6247d3ce075b260db14`, exact version `=0.10.0`, matching every other Host consumer. No local patch/path/vendor dependency is committed.
- Re-ran all 27 tool tests on Runtime 0.10.0. Added explicit root/self delegation rejection (including an already registered member sending delegate:true to itself), without imposing an idle or capacity limit. Full integrated workspace gates remain pending the real core bridge stage.

## Owner-interface refinements

- Preserve name/title from the same atomic config snapshot; submit complete metadata and configuration with the original revision.
- The completion contract separately carries actual consumed input ranges and explicitly discarded inputs. A task discarded before consumption reports `not_executed`, with no invented TurnResult or success outcome.
- Scope cleanup retains failures as actual blockers, caches only success, and permits a later explicit close to continue original owner cleanup. The first close does not automatically retry it.
- Runtime 0.10 focused gate: `cargo test -p minicore-host-tools --locked` has 29 passing tests; `cargo clippy -p minicore-host-tools --all-targets --locked -- -D warnings` and formatting pass.
- A production `core_adapter.rs` and actual Host/Runtime integration tests are being prepared against the sole core owner's capability API. They are not included in the verified build until the corresponding integrated core and observation commits are available.
