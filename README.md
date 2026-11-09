# minicore-host

Multi-agent, multi-session Host backend under development against [SPEC.md](SPEC.md), design v4.93.

The implementation contains strict contracts, Linux durable storage, real Runtime scheduling, production HTTP/SSE resources, scoped observation/interaction adapters and the owner-bound agent tool. It is not yet the complete first version. See [PROJECT.md](PROJECT.md) for exact baselines, implemented boundaries and remaining work.

## Local checks

Rust 1.90.0 is pinned in `rust-toolchain.toml`. Run `./scripts/check.sh` for formatting, Clippy, tests and documentation checks. Dependencies are pinned and the root lock is committed. Whitespace checks include committed changes and require full Git history; CI uses `fetch-depth: 0`.

The Runtime dependency is exactly `0.10.0`, pinned to published commit `69faf9196991524b5549499d6208a1eaf567bf56` on `zqcli/minicore-runtime:feat/host-history-handoff`. It includes authoritative history/partial handoff, application-quota removal and retry-progress observation. No local Runtime patch, copied source or bootstrap bundle is required. Runtime's exact-head CI passed all seven checks; see [PROJECT.md](PROJECT.md) for verification boundaries and the source-tree mapping.

## Components

- [In-process owners](crates/host-core/README.md)
- [Protocol contracts](crates/host-protocol/README.md)
- [Provider adapter](crates/host-provider/README.md)
- [Storage schema](crates/host-storage/README.md)
- [Workspace and change-reading backend](crates/host-workspace/README.md)
- [Public method implementation map](docs/method-coverage.md)
- [Current local executable review handoff](docs/executable-review-handoff.md)
- [Earlier local execution review handoff](docs/execution-review-handoff.md)
- [Published owner/resource review handoff](docs/integration-review-handoff.md)
- [Earlier foundation review handoff](docs/review-handoff.md)
- [Implementation/checkpoint log](tasks/001-foundation/TASK.md)
- [Controller and integration evidence](tasks/005-controllers/TASK.md)

The earlier foundation is published on `dev` at [82583b31ac9091164da5f6dcfee6bb23e05137e5](https://github.com/zqcli/minicore-host/commit/82583b31ac9091164da5f6dcfee6bb23e05137e5), with [exact-head Host CI passing](https://github.com/zqcli/minicore-host/actions/runs/42629644954). The [commit mapping](docs/host-publication.json) translates historical local checkpoints to GitHub commits. Remote `main` remains the README-only ancestor and has not received development changes. Live provider and TUI checks have not been run; external independent review and remaining first-version work are still required.

The later owner, observation, delegation and resource checkpoint is now published at [c6d8a5e1cf1ea52c8e1e4c104efba3774efc2f5e](https://github.com/zqcli/minicore-host/commit/c6d8a5e1cf1ea52c8e1e4c104efba3774efc2f5e). Its [fixed review handoff](docs/integration-review-handoff.md) and [64-commit source mapping](docs/owner-integration-publication.json) identify the exact reviewed code and limitations. Local and independent gates pass 506 unique tests and 3 doctests; [exact-head CI 42650587425](https://github.com/zqcli/minicore-host/actions/runs/42650587425) passed. Later reload/compaction/ordinary-tool/application topics are not included in that remote checkpoint.

The current local executable checkpoint is `e3d634a2b0cd9e662d8efc26284c85af82edd8e5`: real process/configuration startup, production read/write/edit/bash, workspace routes, canonical controls, context and reviewed file-lease corrections. It also includes the reviewed cold history.read/load coordination fix. Its complete gate passes **779 unique tests and 4 doctests**. [The fixed handoff](docs/executable-review-handoff.md) records 50 routed methods, actual binary acceptance and remaining contract findings. This code is not yet published. The most recent verified published documentation checkpoint remains [a84e36f32c9ab7340ac06f5fb3a2cc47d3ab0f0f](https://github.com/zqcli/minicore-host/commit/a84e36f32c9ab7340ac06f5fb3a2cc47d3ab0f0f), with [successful exact-head CI](https://github.com/zqcli/minicore-host/actions/runs/42653342131).

Deletion, public query projection corrections, ordinary-input provenance and complete business diagnostics remain active work. Earlier local checkpoints, including the 650-test execution gate and subsequent reproduced lease failures, remain recorded in their task/handoff files. A passing aggregate is not a claim that the first version or external review is complete.
