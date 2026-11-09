# minicore-host

Multi-agent, multi-session Host backend under development against [SPEC.md](SPEC.md), design v4.93.

The foundation contains strict contracts, Linux durable storage, a production HTTP/SSE model adapter and direct-owner scheduling through real MiniCore Runtime. It is not yet the complete first version. See [PROJECT.md](PROJECT.md) for exact baselines, implemented boundaries and remaining work.

## Local checks

Rust 1.90.0 is pinned in `rust-toolchain.toml`. Run `./scripts/check.sh` for formatting, Clippy, tests and documentation checks. Dependencies are pinned and the root lock is committed. Whitespace checks include committed changes and require full Git history; CI uses `fetch-depth: 0`.

The Runtime dependency is exactly `0.10.0`, pinned to published commit `59ee8ce668f72a21d408e6247d3ce075b260db14` on `zqcli/minicore-runtime:feat/host-history-handoff`. It includes authoritative history/partial handoff and application-quota removal. No local Runtime patch, copied source or bootstrap bundle is required. Runtime's exact-head CI passed all seven checks; see [PROJECT.md](PROJECT.md) for verification boundaries and the source-tree mapping.

## Components

- [In-process owners](crates/host-core/README.md)
- [Protocol contracts](crates/host-protocol/README.md)
- [Provider adapter](crates/host-provider/README.md)
- [Storage schema](crates/host-storage/README.md)
- [Workspace and change-reading backend](crates/host-workspace/README.md)
- [Public method implementation map](docs/method-coverage.md)
- [Self-contained review handoff](docs/review-handoff.md)
- [Implementation/checkpoint log](tasks/001-foundation/TASK.md)
- [Controller and integration evidence](tasks/005-controllers/TASK.md)

The foundation is published on `dev` at [82583b31ac9091164da5f6dcfee6bb23e05137e5](https://github.com/zqcli/minicore-host/commit/82583b31ac9091164da5f6dcfee6bb23e05137e5), with [exact-head Host CI passing](https://github.com/zqcli/minicore-host/actions/runs/42629644954). The [commit mapping](docs/host-publication.json) translates historical local checkpoints to GitHub commits. Remote `main` remains the README-only ancestor and has not received development changes. Live provider and TUI checks have not been run; external independent review and remaining first-version work are still required.
