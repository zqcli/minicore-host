# minicore-host

Current Host workspace version: **0.9.10**, prepared as `v0.9.10` on local `dev`.
Remote publication is awaiting Git authentication in a network-enabled environment.
See [the release record](tasks/102-release-0.9.10/TASK.md) for the source,
version consistency review and validation.

The current simplification unifies owner creation/metadata paths and shares
facts, protocol/SDK templates and test fixtures. See [current status](PROJECT.md)
and [Task101](tasks/101-owner-path-simplification/TASK.md) for its fixed baseline,
intentional Rust API changes and validation. The full local gate passes **955
unique Rust tests, 4 doctests and 28 Python checks**, with strict lint/docs. The
preceding R1-R4 fixes remain in
[Task100](tasks/100-review-r1-r4/TASK.md). The backend is under development against
v4.93; actual external-provider and consuming TUI acceptance remain separate work.

Multi-agent, multi-session Host backend under development against [SPEC.md](SPEC.md), design v4.93.

The implementation contains strict contracts, Linux durable storage, real Runtime scheduling, production HTTP/SSE resources, scoped observation/interaction adapters and the owner-bound agent tool. It is not yet the complete first version. See [PROJECT.md](PROJECT.md) for exact baselines, implemented boundaries and remaining work.

## Tool approval

Execution defaults to Ask. Automated callers can explicitly choose Auto;
ReadOnly permits recognized builtin reads, inspection and waiting. See the
[configuration and migration guide](docs/tool-approval.md). This is not an OS
sandbox, and configured credential exclusion does not isolate same-user processes.

## Local checks

Rust 1.90.0 is pinned in `rust-toolchain.toml`. Run `./scripts/check.sh` for formatting, Clippy, tests and documentation checks. Dependencies are pinned and the root lock is committed. Whitespace checks include committed changes and require full Git history; CI uses `fetch-depth: 0`.

The Runtime dependency is exactly `0.10.0`, pinned to published commit `3aa47629929906832dcb309afb7d242c1ae3e53f` on `zqcli/minicore-runtime:feat/host-history-handoff`. It includes authoritative history/partial handoff, application-quota removal, retry-progress observation and source-measured retry waits in the joined report. No local Runtime patch, copied source or bootstrap bundle is required. Runtime producer and independent local gates pass 324 tests and 2 doctests; its new [exact-head CI](https://github.com/zqcli/minicore-runtime/actions/runs/42746444546) passed. The previous pinned head's CI also passed all seven checks. See [PROJECT.md](PROJECT.md) and [the publication evidence](docs/runtime-retry-wait-publication.json) for fixed boundaries.

## Components

- [Executable startup/configuration](crates/host-app/README.md)
- [Production local tools](crates/host-local-tools/README.md)
- [Client transports](crates/host-transport/README.md)
- [In-process owners](crates/host-core/README.md)
- [Protocol contracts](crates/host-protocol/README.md)
- [Provider adapter](crates/host-provider/README.md)
- [Storage schema](crates/host-storage/README.md)
- [Workspace and change-reading backend](crates/host-workspace/README.md)
- [Current fixed-source method audit](docs/current-contract-audit.md)
- [Historical method implementation map](docs/method-coverage.md)
- [Current local executable review handoff](docs/executable-review-handoff.md)
- [Earlier local execution review handoff](docs/execution-review-handoff.md)
- [Published owner/resource review handoff](docs/integration-review-handoff.md)
- [Earlier foundation review handoff](docs/review-handoff.md)
- [Implementation/checkpoint log](tasks/001-foundation/TASK.md)
- [Controller and integration evidence](tasks/005-controllers/TASK.md)

Historical checkpoints, source mappings, review findings and performance evidence
are indexed in [PROJECT.md](PROJECT.md); detailed evidence remains in its original
task and handoff files.
