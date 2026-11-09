# Task 027: Route production Host resource queries

## Fixed baseline and ownership

- Worktree: `/workspace/shared/minicore-host-resource-transport`; branch: `feat/resource-transport-routes`.
- Integration baseline: `c2ed00eab0422d17eb45b3a310b87a226d2b62f3`.
- Parent bootstrap `7deba95` adds only the transport test dependencies on the real resource assembler and provider; local cherry-pick `d5c49bb`.
- Parent fixture correction `9ace676` and formatting correction `23be155` were taken unchanged as `0fb5d5c` and `cd1ef87`. They repair the pre-existing task024 canonical HostRecord seed, not resource transport behavior.
- Runtime is the published `69faf9196991524b5549499d6208a1eaf567bf56`, version `0.10.0`, with no patch, path override, or copied Runtime implementation.
- SPEC v4.93 remains SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`. TUI stays unchanged at its reference `3beeb1d8a525e58ce6352e05061f242c5580e248`.
- Owned production changes are the new transport resource query DTO, its module export, and additive resource operation/client methods. No WSS, RPC framing, output, interaction, Session, or core configuration implementation changed.
- No remote push, main branch change, CLI process, real credentials, external provider operation, or TUI validation.

## Implementation and exact source checkpoints

- `cb2b7ec`: `feat(transport): route Host resource queries through typed operations`.
- `a728177`: `test(transport): verify production resource queries across connections`.

`host.config.get`, `model.list`, and `tool.list` are real routes in the existing single typed operation table. `HostClient::config`, `models`, and `tools` execute the original Host methods directly in process; stdio and WSS use those same operations through the existing RPC decoder. No parallel resource catalog, cached result store, JSON round-trip on the typed path, or alternate owner entrance is added. `SUPPORTED_METHODS` now has 34 real routes.

`ToolListRequest` omits `target` for all available tools or carries the existing full `AgentRef` / `SessionRef` through `ToolListTarget`. Their disjoint, strict reference types reject mixed/incomplete targets instead of dropping a Host or Session qualifier. An Agent's configured default set and a Session's independently committed set are queried separately. The Agent defaults are not an extra permission ceiling. Missing owners, foreign Host ownership, and removed selected tools retain the original owner's Fault instead of an empty success.

Results preserve lexical ordering, complete same-leaf model references, original Runtime ToolSpec descriptions and input schemas, readable configuration and revision. Private credentials and executable handles do not enter these values. Queries are permitted for read-only clients. No new IDs, item-kind tags, filtering platform, dynamic registration, or requirement to list before executing is introduced.

## Real acceptance coverage

`crates/host-transport/tests/resource_queries.rs` adds **4 unique test cases**. Iterating over three real connection paths does not increase the unique count.

1. Read-only in-process, real OS-pipe stdio, and generated-certificate loopback TLS/WSS clients read identical configured values and original declarations. Two models with the same leaf name preserve distinct full references and ordering. Agent defaults and Session overrides are independently enabled/disabled, including a Session selecting a Host tool absent from its Agent defaults. A real business write remains read-only denied. Query and empty-creation work does not send a provider request. Configuration/result/error values omit both fixture secret sentinels.
2. The actual `HostResourceBuilder` prepares a complete replacement and the core owner publishes its real reload. All adapters observe the new revision and genuine empty available lists. Existing Agent/Session selections remain untouched and return `SessionResourceMissing`, while wrong Host, missing Agent and missing Session retain their precise errors.
3. For each connection path, known model/tool references create a real Session and complete a real Runtime Turn through the production HTTP/SSE provider on literal loopback **before any list query**. The captured request uses the configured provider model and original tool name, description and schema. The final history contains the actual fixture response. This is a named local HTTP fixture, not a live external model result.
4. Literal JSON-RPC over real pipes and loopback TLS has identical result and validation shapes. Extra authority/key fields, incomplete targets, and mixed references fail with `-32602` without echoing the supplied secret. Canonical but unimplemented `host.reload` remains `HostMethodUnavailable`; the nonexistent `host.reload_config` alias remains method-not-found.

## Reload integration boundary

The SPEC and protocol catalog name the method **`host.reload`**, not `host.reload_config`. SPEC sections 2, 6, and the key-rotation contract require operation-correlated facts with the unified receipt envelope and owner-backed `host.receipt.get` lookup even when key rotation closes the originating connection.

The existing `Host::reload_configuration(caller, expected_revision, PreparedHostConfiguration)` is a real prepared publication helper, but returns only `HostReloadResult::Applied` or `RestartRequired`. It takes no operation ID and does not add a reload branch to the protocol Receipt or Host receipt lookup. Wrapping its return in transport-local JSON would therefore not complete the public method. The parent confirmed that a separate core/protocol owner will implement that stage. This task deliberately does not expose a fake or partially conforming reload route.

That next owner needs a concrete reload request, unified HostReloadReceipt (kind, Host identity/incarnation, operation ID and actual result), accepted Host-owned preparation/save/publication facts, and lookup through the current owner receipt path. Independent calls reusing an operation ID must retain the established ambiguity semantics, never become idempotent replay. The real persistent save, key invalidation, abandoned waiter and resource lifetime behavior must remain in the existing Host owner. A new configuration-source platform is not required.

## Verification

Environment used for every build/test command:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-resource-transport-target
```

- Initial focused compile found three test fixture API mistakes: ToolBinding has no options field, Fault exposes a message accessor, and TurnInfo uses id. Those were corrected before tests passed. Log: `/tmp/task027-focused.log`.
- `cargo test -p minicore-host-transport --test resource_queries --locked`: all 4 tests passed. Log: `/tmp/task027-focused2.log`.
- `cargo metadata --format-version 1 --locked --offline` confirms the exact Runtime Git source/revision and version above; `sha256sum SPEC.md` confirms the unchanged baseline.
- Against exact committed source `a728177`, `./scripts/check.sh` passed in full: whitespace, formatting, strict all-target workspace Clippy, all workspace unit/integration targets, 3 doctests, and strict rustdoc. Log: `/tmp/task027-full-gate.log`.
- `cargo test --workspace --all-targets --locked -- --list` reports **477 listed and 477 unique test cases**, deduplicated by test binary and test name. Log: `/tmp/task027-list.log`. Nested storage subprocess output is not added to the count.
- The four new resource tests were repeated three further times; all 12 executions passed. These are repetitions of the same four unique cases, not additional coverage counts. Log: `/tmp/task027-repeat.log`.
- The final task record contains no source changes. Its whitespace and formatting checks pass. No new production test failure occurred in this task; passing these runs does not claim to fix the independently tracked transport-close issue.

## Remaining limits

- `host.reload` is explicitly blocked on its proper core/protocol receipt integration, not hidden behind another method name.
- Production automatic compaction remains explicitly rejected by the resource assembler. Acceptance fixtures deliberately disable it; the normal default is not silently treated as supported.
- The built-in agent tool is real, but ordinary filesystem/shell tool installation and complete CLI/configuration-source loading are outside this query stage.
- Other worktrees own the previously observed intermittent transport-close BrokenPipe and interaction mapping corrections. This task does not suppress I/O errors or modify their code/tests.
- This stage completes three resource query routes, not the whole SPEC's 52-method product or end-to-end TUI integration.
