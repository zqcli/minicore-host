# Reviewed execution integration handoff

## Fixed target and publication status

- Repository: `zqcli/minicore-host`; intended integration branch: `dev`.
- Fixed local code and dependency target: `76a7bfbe7f70d7f707d48cc38190829d14c137f2`, on `stage/reviewed-next-dev`.
- Local comparison base: `810e3817a78855871d13f8127a3f0e87db821d66`, whose exact tree is published at [a84e36f32c9ab7340ac06f5fb3a2cc47d3ab0f0f](https://github.com/zqcli/minicore-host/commit/a84e36f32c9ab7340ac06f5fb3a2cc47d3ab0f0f). The base's [exact-head CI](https://github.com/zqcli/minicore-host/actions/runs/42653342131) passed. [Its mapping](owner-handoff-publication.json) records the verified publication.
- This new execution target is **not yet published**. Local tests are not hosted CI or proof that another checkout can fetch this target.
- [The integration map](execution-linear-map.json) preserves 39 earlier stage commits in order plus the diagnostic file-lease correction. Each earlier code/dependency tree matches its linear counterpart. Only already-published handoff documents were retained during linearization. The final diagnostic source's task note moved from the excluded application task into task 039.
- No published history was replaced. `main` remains `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. Main merge and release remain blocked on complete first-version work and the requested external independent review.

The earlier 23-commit checkpoint `21521919b5ecacea2fd72c698ea7521241a9e2d4` and its [mapping](execution-source-map.json) remain historical evidence. Earlier published owner/resource and foundation targets remain linked from [PROJECT.md](../PROJECT.md).

## Scope and implementation

1. **Canonical reload** (SPEC sections 2, 6, 8 and 10): `host-protocol` reload receipts, `host-core` accepted preparation/publication, and `host-transport` typed/stdio/WSS entry. The accepted owner survives caller loss, retains original receipt facts and only invalidates network authentication after a real committed key change. Tasks 030 and 035.
2. **Compaction engine, persistence and Session ownership** (sections 4, 5 and 8): `host-compaction`, `host-storage` coverage records, `host-core/src/session/compaction*`, and two canonical transport controls. Utility work uses actual selected models, frozen resource binding, estimates labelled as estimates and reported usage left unknown when absent. Native history remains append-only. Effective coverage changes only after the actual save barrier. Cancellation targets the selected operation and never skips required settlement. Tasks 031, 036, 037 and 040.
3. **Standalone ordinary tools** (sections 5 and 7): `host-local-tools` read/write/edit/bash implementations, actual process supervision, and mutation exclusion shared by clones of an actual `Workspace` binding. This is the module checkpoint, **not** production registry/output/edit-owner integration. Agent code is a reference, not a compatibility baseline; the mature process-wrap component is reused without importing old application quotas. Tasks 032 and 038.
4. **Shared diagnostics** (section 12): `host-diagnostics` sanitized structured records, stderr/file sinks, bounded flush observation and an exclusive file lease held by the actual writer through its exit. A timeout does not pretend the worker has stopped or release its lease. No retry destination or global lock service was added. Production process installation and business-event instrumentation are separate. Task 039.

There are **37 actual shared typed/stdio/WSS routes**: the previous 34 plus reload and two compaction controls. The [52-method map](method-coverage.md) separately labels unfinished result schemas and missing handlers. A DTO, route or module is not a completed product feature.

## Scoped independent review

Each production component was independently reviewed before this composition. Counts below belong to their fixed topic trees, not additive subsets of this aggregate:

- Reload owner: 492 unique tests / 3 doctests plus real HTTP resource-swap probes. Reload transport: 576 / 3 plus 14 owner/pipe/TLS probes.
- Compaction engine: 516 / 3 plus 10 probes. Storage: 584 / 3 plus four journal/projection probes.
- Session compaction: original review reproduced mutable original receipt facts and invalid cancellation claims. Both were fixed at source `270f5becc36294006a90dd30735525c3781b4089`; independent re-review passed 608 / 3 and six probes, including the unchanged original failures.
- Compaction transport: 631 / 3; nine real transport cases and five additional probes were repeated ten times, covering abandoned waits, authority, saving, cancellation and ordinary execution.
- Ordinary-tool module: original lost-edit and premature process-group completion findings were fixed. Independent review passed 523 / 3, 29 focused module cases and 15 additional probes. A production binding must reuse the actual Workspace; independently reopening the same path is not a shared lock.
- Diagnostics: pipeline review passed 521 / 4 and seven probes. The later file-lease correction at source `af0f88eff812239fb0152e78cba78a02420e9f37` independently passed 19 focused cases / one doctest and real cross-process, alias, failed-initialization and blocked-FIFO checks.

Tasks retain the exact topic commits, failures, corrections and commands. Those scoped reviews initially closed their findings. A subsequent aggregate run at public-control topic `c4d3cbe14f0c88011057e57cd1b05d9f515de165` exposed a new failure in the unchanged diagnostics test `same_process_and_aliases_are_exclusive_until_actual_worker_completion`: reopening an alias after `ShutdownOutcome::Flushed` returned `LockOutput(WouldBlock)`. The log is `/workspace/shared/public-controls-final-check.log`. The writer already drops its File before signaling completion; concurrent subprocess FD inheritance is being investigated, not yet a proven cause. Publication is held for deterministic reproduction and a separately reviewed correction. The prior successful aggregate is not a claim that this finding is resolved. None of these reviews substitutes for external review or covers later tool/application/workspace topics.

## Exact aggregate verification

At exactly `76a7bfbe7f70d7f707d48cc38190829d14c137f2`, Linux x86_64 and pinned Rust 1.90.0:

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
cargo metadata --locked --offline --format-version 1
sha256sum SPEC.md
```

All succeeded. The script checks committed and worktree whitespace, rustfmt, strict all-target Clippy, workspace tests, doctests and strict rustdoc. The independent inventory gives **650 unique target/test cases and 4 doctests**; nested storage subprocess output is not counted twice. The code and dependency tree is unchanged by this subsequent documentation commit.

Local retained logs are `/workspace/shared/minicore-host-validation/dev-next-76a7bfb-full.log`, `dev-next-76a7bfb-test-list.txt` and `dev-next-76a7bfb-metadata.json`. Their digest manifest is checked in as [execution-verification.json](execution-verification.json); commands and permanent regressions can be rerun from this source.

Tests use real owners, Runtime, temporary storage, actual processes/pipes and local HTTP/SSE/TLS fixtures with explicitly named scripted models. They are not live remote-provider, real Host executable, TUI, performance or capacity measurements.

## Dependencies and explicit limits

- Complete SPEC v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Exactly one Runtime `0.10.0`, published Git `69faf9196991524b5549499d6208a1eaf567bf56`, `feat/host-history-handoff`; no local patch or copied dependency. Its [exact-head CI](https://github.com/zqcli/minicore-runtime/actions/runs/42624211012) passed all seven jobs.
- Runtime upstream dev reference remains `3fd5ecda1fb7c717ab0a39218d90268b4d97909c`.
- Agent reference `f36a87366d8e2fe0a1c8216463887902e471c739` and unchanged TUI `3beeb1d8a525e58ce6352e05061f242c5580e248` still lock Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`. No joint validation is claimed.
- No application capacity quotas, silent truncation, background recovery framework or new persistent output store. Stable local workspace configuration is the intended scope; arbitrary external mutation is not made transactional.
- Saving and cleanup failures remain distinct from business outcome. Bounded diagnostic waiting does not prove worker termination. Controlled shutdown cannot claim stopped while necessary save/cleanup remains incomplete.
- Astra/max workers were requested; no separate actual-runtime configuration attestation is available.

Next: integrate separately reviewed production tools, workspace routes and actual executable; finish controls/context/deletion, prompt association and business diagnostics; then complete process/live-provider/performance acceptance and external review. Keep all changes incremental and preserve these fixed checkpoints.
