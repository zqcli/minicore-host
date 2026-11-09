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
- Full workspace gates and unique `--list` count: pending the next implementation stage.

## Remaining integration

- Core has no admitted-Turn internal operation capability or registered delegation result handoff yet. Parent is assigning its sole owner. The observation output hook runs after claim and must not be used for fallible business preparation.
- Bind selected tool implementations before consuming initial input; preserve the frozen ToolSet across all internal loops.
- Implement scope ownership, standard notification delivery, root cleanup hooks, and real Runtime Tool execution, initially with explicitly named port fixtures.
- Add real owner adapter and Host end-to-end tests once core capabilities land. Do not present fixture tests as real Host delegation.
- Incorporate the observation owner's actual output/page request types rather than invent a parallel read protocol.
- Metadata updates require atomic owner name/title plus config revision commits; unsupported paths must fail explicitly.
- Runtime quota-removal publication is in progress; current crate revision remains the existing reproducible `18a6ac9b82d003330cd870275efca793cdcc9a96` (0.9.2). No unpublished local SHA is presented as remote-reproducible.
