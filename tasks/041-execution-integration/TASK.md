# Integrate reload, storage and local tool modules

Status: isolated integration passed; not yet published or first-version complete. Follow AGENTS.md.

## Fixed checkpoint and scope

- Branch: `stage/next-reviewed`; code `21521919b5ecacea2fd72c698ea7521241a9e2d4`.
- Compare with local dev `705a3e8cfc14d56e51fc08e522df04ee0adca1f6`, kept frozen for its separate publication. No main change or force update.
- The 23 granular source commits and their integrated counterparts are in `docs/execution-source-map.json`; raw subjects/messages and order are retained.
- Tasks 030/035 provide canonical reload ownership and typed/stdio/WSS routes. Tasks 031/036 provide the pure compaction engine and append-only, validated storage coverage/projection. Tasks 032/038 provide read/edit/write/bash modules and the actual process-group and shared-Workspace mutation corrections.
- Runtime remains the single published `0.10.0` package at `69faf9196991524b5549499d6208a1eaf567bf56`. SPEC hash and Agent/TUI references remain unchanged. No live-provider or TUI integration is claimed.

## Decisions and independent review

Only the task023 test-file tail conflicted while integrating source a80311c: both the existing Session-resource-resume module and the production reload-preparer test were retained. No assertion was removed. Other production changes applied without conflict.

Each component had an independent scoped review before this composition. Reload review reproduced 492/3 and real HTTP resource-swap probes; transport review reproduced 576/3 and 14 owner/stdio/WSS probes. Compaction engine review passed 516/3 plus 10 probes; storage review passed 584/3 plus four history/journal probes. Local-tool correction review passed 523/3 plus 15 probes, closing the original shared-binding lost-edit and false process-group cleanup findings. Those differing counts describe their exact topic trees, not this combined checkpoint. Reproducible regression cases and fixed source hashes are in the corresponding task records.

The ordinary-tool Workspace lock is shared by clones of an actual binding. Independently reopening a path is not equivalent; production binding reuse remains required. Process cleanup uses the referenced mature process-wrap component without importing old Agent quotas or lifecycle ownership. Agent code is a reference, not a compatibility baseline.

## Actual verification

At exactly 2152191, on Linux x86_64 with pinned Rust 1.90.0:

```sh
cargo check --workspace --all-targets --locked
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
cargo metadata --locked --offline --format-version 1
sha256sum SPEC.md
```

All commands pass: committed and uncommitted whitespace, formatting, strict all-target Clippy, **602 unique target/test cases**, **3 doctests**, and strict rustdoc. The metadata contains exactly one Runtime package at the pin above. SPEC remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

Local evidence: `/workspace/shared/minicore-host-validation/next-2152191-check.log`, `next-2152191-full.log`, `next-2152191-test-list.txt`, and `next-2152191-metadata.json`. Tests use real owners/Runtime, temporary files, processes, pipes and local HTTP/TLS fixtures; these are not live-model, executable-startup or performance results.

## Remaining work

- Session manual/automatic compaction owner integration is not included. Its separate 606/3 gate passed, but independent probes found mutable original control receipts and invalid claimed-cancel facts; both must be corrected and re-reviewed before integration.
- Ordinary tools still need production registry/resource binding, actual Session-bound output/change retention and shared Workspace reuse. No standalone no-op sink may become the production write path. Tool output does not require a new permanent output store.
- Compaction transport, diagnostics integration, canonical workspace and remaining methods, executable startup, final process/live-provider/performance acceptance and external review remain unfinished. TUI is unchanged.
- The older dev publication is separate and must be confirmed by exact remote SHA/CI before claiming a GitHub handoff. This checkpoint has no remote publication yet.

## Superseding reviewed execution checkpoint

The preceding 2152191 record remains a historical 23-commit checkpoint. Current
integrated code is `76a7bfbe7f70d7f707d48cc38190829d14c137f2` on
`stage/reviewed-next-dev`, based on local `810e3817a78855871d13f8127a3f0e87db821d66`
(published as `a84e36f32c9ab7340ac06f5fb3a2cc47d3ab0f0f`). The new
`docs/execution-linear-map.json` preserves 39 preceding source commits and one
separately reviewed diagnostics file-lease correction. Published handoff docs
remain intact; no published history, main or TUI was rewritten.

Both Session compaction findings above were corrected and independently
re-reviewed before integration. Manual/automatic owner/storage integration and
both typed/stdio/WSS controls are now included, together with diagnostics and
its actual writer-held OS lease. Standalone ordinary tools are included, but
production binding, actual Session edit/output ownership, canonical workspace
routes and executable startup remain separate topics.

At exactly 76a7bfb, `./scripts/check.sh`, all-target `--list`, offline Cargo
metadata and SPEC hashing pass: **650 unique cases / 4 doctests**, strict Clippy,
rustdoc, fmt and committed/worktree whitespace. Runtime remains the sole
published `0.10.0` Git package at `69faf919`; SPEC digest is unchanged. Full logs
and inventory are retained under `/workspace/shared/minicore-host-validation/`
as `dev-next-76a7bfb-full.log`, `dev-next-76a7bfb-test-list.txt` and
`dev-next-76a7bfb-metadata.json`. Their hashes are in
`docs/execution-verification.json`.

The self-contained `docs/execution-review-handoff.md` gives fixed source/base,
component review evidence, exact commands, decisions, limits and next work.
There are 37 routed methods, not 52 finished contracts. This target is not yet
published and no exact-head hosted CI is claimed for it. A later publication
must record its actual SHA, source mapping and CI before GitHub handoff.

### Subsequent aggregate finding

A later public-controls aggregate at `c4d3cbe14f0c88011057e57cd1b05d9f515de165`
failed the unchanged diagnostic alias-reopen test: `Flushed` was observed, then
reopening returned `LockOutput(WouldBlock)` at file_lease.rs:50. The complete
log is `/workspace/shared/public-controls-final-check.log`. Publication of this
checkpoint is held for deterministic investigation and correction in task 047.
The earlier 650/4 pass is preserved but does not close this new finding. No
retry, sleep, production-lock weakening or test serialization is assumed.
