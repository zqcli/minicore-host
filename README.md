# minicore-host

Multi-agent, multi-session Host backend under development against [SPEC.md](SPEC.md), design v4.93.

The foundation contains strict contracts, Linux durable storage, a production HTTP/SSE model adapter and direct-owner scheduling through real MiniCore Runtime. It is not yet the complete first version. See [PROJECT.md](PROJECT.md) for exact baselines, implemented boundaries and remaining work.

## Local checks

Rust 1.90.0 is pinned in `rust-toolchain.toml`. Run `./scripts/check.sh` for formatting, Clippy, tests and documentation checks. Dependencies are pinned and the root lock is committed. Whitespace checks include committed changes and require full Git history; CI uses `fetch-depth: 0`.

The required Runtime history/partial extension is pinned to published commit `18a6ac9b82d003330cd870275efca793cdcc9a96` on `zqcli/minicore-runtime:feat/host-history-handoff`. No local Runtime patch, copied source or bootstrap bundle is required. Runtime's push CI passed all seven checks; see [PROJECT.md](PROJECT.md) for verification boundaries and the exact source-tree mapping.

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

Live provider and TUI checks have not been run. `main` is not merged with development changes; Host has not been remotely published. The Runtime prerequisite branch is published.
