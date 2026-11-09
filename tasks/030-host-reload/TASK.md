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
- Focused protocol, owner lifecycle and production HTTP/SSE validation, strict lint/docs and full workspace gate are pending. No live provider, real credentials or completed transport-route validation is claimed.
- All checks use a separate `/workspace/shared/minicore-host-reload-target`, the shared fixed Rust/Cargo installations, two build jobs, no incremental compilation and disabled dev/test debug info.

## Next

Complete focused acceptance/failure/cleanup/provider tests, run the full repository gate and record exact source commits/results. The original transport owner will integrate canonical typed/stdio/WSS host.reload using this frozen Host entry point and the existing receipt lookup after core review.
