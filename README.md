# minicore-host

Multi-agent, multi-session Host backend under development against [SPEC.md](SPEC.md), design v4.93.

The implementation contains strict contracts, Linux durable storage, real Runtime
scheduling, production HTTP/SSE resources, scoped observation/interaction adapters
and the owner-bound agent tool. Start with the [executable guide](crates/host-app/README.md).
See [current project status](PROJECT.md) for the release version, exact source,
validation, publication and remaining acceptance work. Historical reports below
describe their fixed checkpoints, not the current open-work list.

## Tool approval

Execution defaults to Ask. Automated callers can explicitly choose Auto;
ReadOnly permits recognized builtin reads, inspection and waiting. See the
[configuration and migration guide](docs/tool-approval.md). This is not an OS
sandbox, and configured credential exclusion does not isolate same-user processes.

## Local checks

See [native CI and distribution packages](docs/ci.md) for platform jobs, release
archives and the fixed Runtime library compiled into each Host executable.
The initial configuration template is [example.host.json](example.host.json).

Rust 1.90.0 is pinned in `rust-toolchain.toml`. Run `./scripts/check.sh` for formatting, Clippy, tests and documentation checks. Dependencies are pinned and the root lock is committed. Whitespace checks include committed changes and require full Git history; CI uses `fetch-depth: 0`.

Shared package metadata and dependency pins are declared in the root
[Cargo.toml](Cargo.toml); members retain their own feature selections. The
committed lockfile fixes the resolved graph. The exact Runtime source and its
validation are recorded in [PROJECT.md](PROJECT.md). No local Runtime patch,
copied source or bootstrap bundle is required.

## Components

- [Executable startup/configuration](crates/host-app/README.md)
- [Production resource composition](crates/host-resources/README.md)
- [Agent tools and owner binding](crates/host-tools/README.md)
- [Production local tools](crates/host-local-tools/README.md)
- [Client transports](crates/host-transport/README.md)
- [In-process owners](crates/host-core/README.md)
- [Protocol contracts](crates/host-protocol/README.md)
- [Provider adapter](crates/host-provider/README.md)
- [Storage schema](crates/host-storage/README.md)
- [Workspace and change-reading backend](crates/host-workspace/README.md)
- [Compaction engine](crates/host-compaction/README.md)
- [Structured diagnostics](crates/host-diagnostics/README.md)

## Project records

Historical checkpoints, source mappings, review findings and performance evidence
are indexed in [PROJECT.md](PROJECT.md#historical-review-index). Detailed evidence
remains in its original task and handoff files. The organization implementation
and release work is recorded in [Task103](tasks/103-project-organization/TASK.md).
