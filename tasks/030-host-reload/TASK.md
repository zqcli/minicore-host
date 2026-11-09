# Task 030: Accept canonical Host reloads and retain owner facts

## Baseline and scope

- Worktree: `/workspace/shared/minicore-host-reload`; branch: `feat/host-reload-receipts`.
- Fixed baseline: `d107462e1e67b2270c268a0b4f51bb4ba01bde00`.
- Sources: unchanged SPEC v4.93, especially §§2, 6, 8, 10; task 023's existing configuration reservation, confirmed-save/publication path and static resource assembler. Follow repository AGENTS.md, including English `type(scope): summary` commits without co-author trailers.
- Runtime remains published `69faf9196991524b5549499d6208a1eaf567bf56`, exactly 0.10.0, without a patch/path override or copied implementation. TUI reference remains unchanged at `3beeb1d8a525e58ce6352e05061f242c5580e248`.
- Scoped changes: protocol reload requests/receipts, core ConfigurationOwner and Host reload/query path, initialization-only production preparer binding, focused tests. No Session, metadata, interaction, observation, transport routes, workspace manifests, CLI or TUI changes. No Git push.

## Contract

`HostReloadRequest<HostConfig>` carries a caller-supplied operation_id, expected_revision and the full readable candidate. Canonical `Host::reload` checks actual connection operation rights, Host lifecycle and configuration reservation before accepting. Host then owns preparation, necessary saving, publication and cleanup independently of the response waiter and original connection. HostResourceBuilder supplies a static preparer through the initial PreparedHostConfiguration; reload cannot install another registry/preparer.

Each accepted invocation has one actual configuration-owner record. Its preparing/applying/applied/restart_required/failed state is returned by cloning the current facts. The original returned value is never mutated or returned twice; a later host.receipt.get is a new read of the original direct owner's current facts. Duplicate operation IDs remain distinct invocations and make the existing single-result lookup ambiguous. There is no generic Operation entity, transport result cache, request replay, automatic retry, repair endpoint or persistence recovery loop.

The existing low-level reload_configuration remains an embedding convenience and shares the same reservation/save/publication implementation. Actual changed restart fields reject the whole candidate. Preparation failure leaves all live state untouched; failed saves retain their real blocking fault and original candidate. Only successful whole publication changes network authentication. Existing Turns keep their actual old resources.

## Validation status

- `cargo check --workspace --all-targets --locked`: passed after the initial implementation.
- Four strict protocol tests pass. They reject absent/invalid correlation and revision fields, caller-supplied authority/wait options, method aliases, unrelated object identities and inconsistent phase fields, while preserving each actual phase and its original method/operation.
- Thirteen configuration owner tests pass (nine new canonical cases and four existing publication/binding cases). Explicit scheduling fixtures gate real preparation, storage and resource destruction. Coverage includes abandoned caller wait, network ConnectionHandle close/revocation, read-only/stale/lifecycle rejection before preparation, force waiting through preparation/save/cleanup, immutable returned snapshots, independent duplicate IDs, retained preparation/save failure facts, all-or-nothing restart classification, unchanged restart settings and initialization-only preparer identity.
- Production resources test target passes ten cases, including two new canonical cases. Real Runtime and production HTTP/SSE on loopback show an already-accepted network-origin Turn continuing with actual A clients/tools/Role after key rotation, a new-key connection reading the original operation, and the next Turn binding B. The credential adapter is explicitly a test fixture; model requests, SSE parsing and tool execution are real components. Default builder initialization supplies the production preparer without a setter.
- First full `./scripts/check.sh` passed all stages: whitespace, formatting, strict workspace all-target Clippy, unit/integration targets, three doctests and strict rustdoc. Log: `/tmp/task030-full-gate.log`. The stronger network-origin fixture and preparer-identity test added afterward were covered by the separate exact-source revalidation below, not retroactively attributed to this first pass.
- Initial owner test compilation found a test-only FaultKind spelling error; it was corrected and the focused target rerun successfully. No production failing test was suppressed.
- No live provider, real credentials, actual WSS/stdio reload route, CLI/process or TUI validation is claimed at this core checkpoint. The new network acceptance cases use actual core connection authority, not a new transport implementation.
- All checks use a separate `/workspace/shared/minicore-host-reload-target`, the shared fixed Rust/Cargo installations, two build jobs, no incremental compilation and disabled dev/test debug info.

## Next

This scoped core/production-preparer implementation and its verification are complete. The integration owner maintains the aggregate PROJECT.md checkpoint. The original transport owner will integrate canonical typed/stdio/WSS host.reload using this frozen Host entry point and existing receipt lookup, then test actual transport timeout/disconnect and new-key receipt lookup. Independent review and that transport integration are separate remaining work; this task does not claim the full 52-method product is complete.

## Frozen integration surface

- `Host::reload(&Arc<Self>, &CallerContext, HostReloadRequest<HostConfig>) -> Result<HostReloadReceipt, Fault>` is synchronous acceptance, not a full-I/O waiter.
- `HostConfigurationPreparer::prepare(&self, HostConfig) -> HostConfigurationFuture<'_>` is a Send future returning a complete PreparedHostConfiguration.
- `PreparedHostConfiguration::with_preparer(Arc<dyn HostConfigurationPreparer>)` supplies the static application assembler only at Host construction. Unassembled low-level Hosts report HostMethodUnavailable for canonical reload.
- `HostResourceBuilder` clones share exactly the original Arc ToolRegistry. Private explicit credentials affect their candidate only; the production reload preparer reads the named environment sources. Embeddings with another source bind their own preparer at construction.
- `Receipt::HostReload` participates in existing direct-owner lookup. `preparing` and `applying` carry the original effective config_revision; `applied` is recorded at the atomic whole-publication/key-invalidation boundary with the new revision. Actual old-resource destruction remains inside the shutdown barrier after publication, without holding owner locks.
- Protocol checkpoint: `e0899cf5b82427894edd353bc86f3dc64b111d63` (`feat(protocol): define canonical Host reload receipts`). Functional source and final checks are recorded below.

## Final source and exact validation

- Functional source: `a80311c10c1d6cb8b0634034e929aadca4d93994` (`feat(host): own accepted reload preparation and receipt facts`), after the protocol checkpoint above. Both commits are based on the fixed d107462 baseline and contain no manifest, lockfile, Runtime source, Session owner, transport or TUI change.
- At exactly a80311c, `./scripts/check.sh` passed in full: whitespace/fmt, strict all-target workspace Clippy, every unit/integration target, three doctests and strict rustdoc. Log: `/tmp/task030-final-source-gate.log`.
- `cargo test --workspace --all-targets --locked -- --list` reports **492 listed and 492 unique target/test cases**. Nested storage subprocess output is not counted again. Log: `/tmp/task030-final-list.log`.
- `cargo test -p minicore-host-core --lib configuration:: --locked`: all 13 owner tests passed; `cargo test -p minicore-host-resources --test task023 --locked`: all 10 production-resource cases passed; `cargo test -p minicore-host-protocol --test reload --locked`: all four strict protocol cases passed. Logs: `/tmp/task030-core3.log`, `/tmp/task030-production2.log`, `/tmp/task030-protocol2.log`.
- After the full gate, the nine new canonical owner cases (`cargo test -p minicore-host-core --lib configuration::reload_tests --locked`) and actual network-origin production HTTP/SSE case (`cargo test -p minicore-host-resources --test task023 canonical_reload_receipt --locked`) passed **three consecutive repetitions** each. Log: `/tmp/task030-final-repeats.log`.
- `cargo metadata --format-version 1 --locked` confirms Runtime `0.10.0` from `git+https://github.com/zqcli/minicore-runtime?rev=69faf9196991524b5549499d6208a1eaf567bf56#69faf9196991524b5549499d6208a1eaf567bf56`. No local patch or path override is used. `sha256sum SPEC.md` still reports `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- This final follow-up commit changes only this task report. No push was performed.

Reproducible environment for the commands above:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-reload-target
```
