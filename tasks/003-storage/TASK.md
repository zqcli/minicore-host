# Implement durable Host storage

Status: active. First storage implementation and 21 focused tests are complete; additional native-history and real OS failure tests, final lint, documentation and integration handoff remain.

## Scope and baseline

- Implements SPEC v4.93 §4 and the storage decisions in §15. The original SPEC.md is unchanged.
- Owns only `crates/host-storage/**` and this task record. Protocol contracts come from `minicore-host-protocol`; native history and authoritative partials come from the Runtime patch based on `3fd5ecda1fb7c717ab0a39218d90268b4d97909c`.
- Linux is the explicit supported storage platform. Other platforms fail compilation rather than silently using weaker locks or path checks.
- Real directory-inode advisory lock, no stale-marker ownership; fd-relative no-follow filesystem operations; owned regular files and non-writable-by-others directories; strict schema/identity validation; atomic object replacement with file and directory sync.
- Session-local JSONL positions, distinct consumed Input, exact Runtime HistoryItem, ModelPartial and TurnFinished facts. Memory creates no object or history files. Loading never restores execution or repairs data.
- Accepted and synchronized positions are separate. Failed saves retain the original byte range; explicit same-run reconciliation permits only exact complete-suffix sync or a proven-unwritten original range. Partial/changed data stays blocked.

## Commands and first milestone results

Environment: `export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo PATH=/workspace/shared/cargo/bin:$PATH`.

Independent-worktree commands use the actual Runtime patch through:

`--config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-handoff"'`

- `cargo check --manifest-path crates/host-storage/Cargo.toml` with that patch: passed.
- `cargo test --manifest-path crates/host-storage/Cargo.toml` with that patch: 4 unit tests and 17 integration tests passed, plus doc tests.
- `cargo fmt --manifest-path crates/host-storage/Cargo.toml`: completed.
- First Clippy run identified a large enum variant and a copy-type clone; both corrected. Final lint run follows.

The local protocol symlink is test scaffolding, not committed. Root workspace/Cargo.lock integration belongs to the parent. No remote writes, live provider execution, TUI validation, power-loss experiment, or cross-platform validation has been performed.

## Hardening milestone

- First implementation commit: `0c32443` (`feat(storage): add durable owner-scoped object and history storage`).
- Added actual kernel `EFBIG` subprocess tests for atomic object-save failure and partial JSONL append. Existing object bytes survive; partial history remains blocked and unchanged after reopening.
- Added full native tool-call/result association, missing-result history, authoritative partial roundtrip, moved-directory, active-writer deletion, and lossy/native unknown-null rejection tests.
- The tests found that serde callbacks can miss fields buffered inside tagged enums. Removed that dependency and made the lossless decode check reject every discarded field, including null; no stored text is silently normalized.
- Latest checks: 4 unit tests + 27 integration tests passed; Clippy `--all-targets -- -D warnings` passed; rustdoc with `RUSTDOCFLAGS='-D warnings'` passed. Final format and task handoff follow.
