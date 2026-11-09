# minicore-host

Multi-agent, multi-session Host backend under development against [SPEC.md](SPEC.md), design v4.93.

The foundation contains strict contracts, Linux durable storage, a production HTTP/SSE model adapter and direct-owner scheduling through real MiniCore Runtime. It is not yet the complete first version. See [PROJECT.md](PROJECT.md) for exact baselines, implemented boundaries and remaining work.

## Local checks

Rust 1.90.0 is pinned in `rust-toolchain.toml`. Run `./scripts/check.sh` for formatting, Clippy, tests and documentation checks. Dependencies are pinned and the root lock is committed.

**Current prerequisite:** the independently committed Runtime extension is still unpublished. The locally verified build uses an ignored Cargo patch to the Runtime worktree at the exact commit in `PROJECT.md`. The upstream revision alone does not supply the required history/partial APIs. Remote standalone reproducibility remains blocked until that extension is reviewed, approved for publication and pinned here. No fallback fake Runtime is provided.

## Components

- [Protocol contracts](crates/host-protocol/README.md)
- [Provider adapter](crates/host-provider/README.md)
- [Implementation/checkpoint log](tasks/001-foundation/TASK.md)
- [Controller and integration evidence](tasks/005-controllers/TASK.md)

Live provider and TUI checks have not been run. `main` is not merged with development changes; no remote publication has been performed.
