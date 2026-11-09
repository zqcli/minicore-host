# Task 023: Assemble production Host resources and configuration

## Baseline and boundaries

- Worktree: `/workspace/shared/minicore-host-resources`; branch: `feat/host-resource-configuration`.
- Fixed baseline: `57687b284fac35649972d236c7d1cce36b6b6178`.
- Runtime: published `69faf9196991524b5549499d6208a1eaf567bf56`, version `0.10.0`; no local patch, path override or copied implementation.
- SPEC v4.93 remains SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- TUI reference stays unchanged at `3beeb1d8a525e58ce6352e05061f242c5580e248`. No live provider, real credentials, CLI/process or TUI validation. No Git push.
- Scoped files: new core configuration module and Host entry points, production `host-resources`, independent tests. No Session/controller/history/interaction internals, tools implementation, or transport production routes changed.
- The one coordinated existing fixture edit converts its direct HostRecord seed to HostConfig. HostConfigInvalid 1601 and HostConfigConflict 1602 are the agreed narrow protocol additions.

## Implementation

The static `HostResourceBuilder` builds real immutable OpenAI-compatible HTTP/SSE clients using the exact configured ModelRef, capability declarations and explicit credential sources. It combines the existing ToolRegistry/CoreTurnPort with the actual selected ToolSet. The builder persists across reloads so independent agent-tool scopes are not replaced. Canonical Workspace binding never changes process cwd. Runtime deadlines and bounded retry settings are mapped explicitly. Each Turn gets exactly one framework text and its frozen Role text, with optional workspace context, without adding them to history.

Host owns one versioned configuration/resource publication. Preparation is outside owner locks. A prepared candidate includes readable values, actual registered declarations, private resource handles and a private access key; only readable values are saved or queried. Model and tool descriptions preserve full ModelRef and original ToolSpec without new IDs. Queries are lexically ordered and can filter the actual Agent/Session tool selection.

All persistent constructors use one HostRecord<HostConfig> format, identity and direct-owner discovery path. Name-only HostSettings is only a Rust embedding convenience, not a second disk format. Existing saved values win over supplied defaults. There is no old-record migration or config inheritance layer.

Reload compares actual listener/logging changes. Any such restart-required change rejects the whole candidate; unchanged restart fields do not block hot changes. There is no logging subscriber reinstallation. Acceptance checks the real caller's operation right and reserves the original revision. Necessary record I/O runs outside all owner/connection locks in a Host-owned task. Abandoning the wait or original connection does not revoke accepted work. After the confirmed save, an accepted-owner continuation publishes the whole snapshot and network key at the existing network boundary. Only actual key changes revoke network callers; local callers and accepted Turns continue. Rejected/prepared and old published resources are dropped outside state locks.

The Host shutdown barrier includes accepted configuration saves and their actual cleanup. A failed save retains the original candidate/fault, does not alter live configuration or authentication, and prevents false stopped. No generic lease, rollback, retry or repair framework was added. The root owns that failed work rather than interpreting readable bytes as a confirmed save.

A resource resolver snapshots one version, prepares outside the lock, and rejects a version change before returning a binding. Bound execution keeps actual old Arc model/tools/prompt resources. Removing resources never traverses or edits existing Session configurations. Missing resources or failed binding leave accepted messages queued and do not fabricate a Turn/history.

## Core interfaces for the next transport owner

- `Host::memory_configured(PreparedHostConfiguration)`.
- `Host::open_configured(path, initial: HostConfig, prepare)` loads the authoritative record before preparation.
- `Host::config_get(&CallerContext) -> HostConfigSnapshot`.
- `Host::model_list(&CallerContext) -> Vec<ModelDescription>`.
- `Host::tool_list`, `tool_list_for_agent`, `tool_list_for_session` return actual `ToolSpec` selections.
- `Host::reload_configuration(&Arc<Self>, &CallerContext, expected_revision, PreparedHostConfiguration) -> HostReloadResult` is the narrow prepared-publication entry, returning Applied or RestartRequired with precise fields. It does not create a fake internal connection.
- `HostResourceBuilder::prepare(HostConfig)` reads only explicitly named credential environment values; `prepare_with_credentials` supports explicit private embedding/test inputs.

These library entries are not completed `host.reload` transport/operation-receipt routes, pagination adapters, CLI/config-source loading or the whole 52-method product. Those require their actual owning integration step.

## Validation and observed failures

Environment for every command:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-resources-target
```

- Initial compile passes caught a private Runtime Text::shared accessor and a test fixture's outdated Tool method name; both were corrected before tests passed.
- A real removed-model test demonstrated that current Session resume returns its existing resource fault even after Host resources are restored. The supported explicit Session update to the repaired reference clears SessionResourceMissing and consumes the original message once; no resend is used.
- `cargo test -p minicore-host-resources --test task023 --locked`: 8 tests passed. These use actual Host/Runtime, production reqwest HTTP/SSE on literal loopback, static tool fixtures and temporary workspace directories. They cover A staying bound across two old-Turn requests while B applies next Turn, key rotation during accepted execution, matching tool declaration/executor selection, framework/Role counts and preserved history, removal and binding failures retaining queued inputs, explicit correction, restart/invalid/stale/read-only atomic rejection, secret-free queries, selected tool queries, persistent reopen, and explicit unsupported compaction/reasoning.
- `cargo test -p minicore-host-core --lib configuration::tests --locked`: 4 tests passed. Explicit scheduling gates wrap the real acceptance/storage/publication owners, not fake successful storage. Coverage: abandoned waiter + disconnected caller + force shutdown during accepted save; a competing revision never writes/publishes; actual host.json filesystem failure preserves live config/key and blocks false stopped; resource preparation returning from an obsolete Host revision does not consume input.
- `cargo fmt --all -- --check`, all-target workspace Clippy with `-D warnings`, three workspace doctests and strict rustdoc pass.
- `cargo test --workspace --all-targets --locked -- --list` reports **438 unique test cases** at this topic checkpoint. Nested storage probe output is not added to that count.
- First `./scripts/check.sh` failed in the untouched transport test `observed_and_unobserved_runs_save_identical_history_and_cold_rpc_reads_do_not_create_runtime`, at observation_tools.rs:296 when unwrapping the server after client close: OS BrokenPipe. Log: `/tmp/task023-full-gate.log`.
- The exact transport test was independently repeated; attempts 1–3 passed, attempt 4 reproduced BrokenPipe, so attempt 5 was not run. Log: `/tmp/task023-transport-repeat.log`. The transport owner was notified. No transport change or blanket I/O-error suppression was made.
- The second full `./scripts/check.sh` passed, including all targets, docs and lint: `/tmp/task023-full-gate2.log`. This does not establish that the intermittent transport-close failure is fixed.
- Final code checkpoint and final revalidation are recorded in the follow-up section below.
- `cargo metadata --format-version 1 --locked` confirms exactly the published Runtime Git source above, with no local patch; `sha256sum SPEC.md` confirms the unchanged design hash.

## Explicit remaining integration limits

- Automatic compaction is not implemented in the current Session owner. Enabled compaction is explicitly rejected by this production assembler. Session defaults currently enable it, so unchanged defaults are not a completed production configuration. Compaction remains a separate owner integration, not a silently ignored setting.
- The built-in registry currently includes the real agent tool. This stage does not install ordinary read/edit/shell modules. Workspace is really bound and retained for the Turn/context, but ordinary tools and canonical workspace query routing are separate work.
- Historical incomplete-tool projection has a separate pure helper task. It has one straightforward production prompt call site to integrate; this topic does not rewrite history or synthesize tool results.
- Existing Session update clears ResourceMissing but not ConfigInvalid/ConfigConflict startup blockers. The parent was notified with the exact owner boundary; those internals remain untouched here. No successful repair of those two blocker kinds is claimed.
- Failed Host record saves retain an explicit blocking fault and original candidate. This stage introduces no public continuation/repair interface.
- Logs/listener initialization and their real CLI process are not implemented here. Their settings are restart-classified metadata; no subscriber/listener is installed by reload.
- No completed transport route, operation-id receipt lookup, broad list query platform, capability negotiation, resource quota or dynamic plugin platform is claimed or introduced.

## Final fixed checkpoint

- Bootstrap: `bfa802e` (`build(resources): add production resource assembly crate`), the independent workspace member/manifest/lock commit. Earlier bootstrap hashes mentioned during development were superseded before integration.
- Functional source: `a510d040f268d459ab2b1972088dfc1265577916` (`feat(resources): assemble versioned Host execution configuration`).
- Against that exact committed source, `./scripts/check.sh` passed in full: whitespace, fmt, strict all-target workspace Clippy, all workspace unit/integration targets, 3 doctests and strict rustdoc. Log: `/tmp/task023-final-a510d04.log`.
- A subsequent exact-source `cargo test --workspace --all-targets --locked -- --list` plus target/test-name deduplication confirmed **438 listed and 438 unique cases**. Log: `/tmp/task023-list-a510d04.log`.
- The earlier transport BrokenPipe remains an observed intermittent failure, not a fixed defect. Its original owner is handling a separate gated teardown fix. The later all-green run is accurately reported without erasing the earlier reproduction.
- The parent will integrate the separate `prepare_prompt_history` helper at the one production prompt call site, add the precise Session config-blocker correction, and own aggregate review/integration. Those commits are deliberately absent from this fixed-base topic.
- No new tool registry platform, application capacity/retention budget, private key in records/queries/Debug, local Runtime patch, TUI edit, remote push, or full-product completion claim was introduced.
