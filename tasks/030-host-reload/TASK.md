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
- First full `./scripts/check.sh` passed all stages: whitespace, formatting, strict workspace all-target Clippy, unit/integration targets, three doctests and strict rustdoc. Log: `/tmp/task030-full-gate.log`. A subsequent stronger network-origin fixture and the preparer-identity test are being rerun at the final source checkpoint; this first pass is not reported as covering those later test edits.
- Initial owner test compilation found a test-only FaultKind spelling error; it was corrected and the focused target rerun successfully. No production failing test was suppressed.
- No live provider, real credentials, actual WSS/stdio reload route, CLI/process or TUI validation is claimed at this core checkpoint. The new network acceptance cases use actual core connection authority, not a new transport implementation.
- All checks use a separate `/workspace/shared/minicore-host-reload-target`, the shared fixed Rust/Cargo installations, two build jobs, no incremental compilation and disabled dev/test debug info.

## Next

Finish final source revalidation and record exact source commits/results. The integration owner maintains the aggregate PROJECT.md checkpoint. The original transport owner will integrate canonical typed/stdio/WSS host.reload using this frozen Host entry point and the existing receipt lookup after core review.

## Frozen integration surface

- `Host::reload(&Arc<Self>, &CallerContext, HostReloadRequest<HostConfig>) -> Result<HostReloadReceipt, Fault>` is synchronous acceptance, not a full-I/O waiter.
- `HostConfigurationPreparer::prepare(&self, HostConfig) -> HostConfigurationFuture<'_>` is a Send future returning a complete PreparedHostConfiguration.
- `PreparedHostConfiguration::with_preparer(Arc<dyn HostConfigurationPreparer>)` supplies the static application assembler only at Host construction. Unassembled low-level Hosts report HostMethodUnavailable for canonical reload.
- `HostResourceBuilder` clones share exactly the original Arc ToolRegistry. Private explicit credentials affect their candidate only; the production reload preparer reads the named environment sources. Embeddings with another source bind their own preparer at construction.
- `Receipt::HostReload` participates in existing direct-owner lookup. `preparing` and `applying` carry the original effective config_revision; `applied` is recorded at the atomic whole-publication/key-invalidation boundary with the new revision. Actual old-resource destruction remains inside the shutdown barrier after publication, without holding owner locks.
- Protocol checkpoint: `e0899cf5b82427894edd353bc86f3dc64b111d63` (`feat(protocol): define canonical Host reload receipts`). Functional source and final checks are recorded below after commit.
