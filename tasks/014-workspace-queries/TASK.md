# Implement workspace queries

Status: implementation and focused tests complete; final whole-workspace validation is in progress. Canonical Session/tool integration remains a separate stage.

## Scope and provenance

- SPEC v4.93 §§2, 7–9, 11 and 13; baseline `413916cac0730d291e5c8163cdc3d59e5dbedcd1`.
- Reference implementation read at `minicore-agent:f36a87366d8e2fe0a1c8216463887902e471c739`: `src/workspace*`, `src/changes.rs`, `src/diff.rs`, and `src/tools/file_change.rs`. Current pinned TUI's workspace/change DTOs informed path/kind, Git XY, hunk and line fields. TUI is unchanged.
- Runtime stays locked at `18a6ac9b82d003330cd870275efca793cdcc9a96`. This crate needs no Runtime dependency or public protocol changes. No version negotiation, storage migration, defensive recovery framework, application capacity quota or owner-lock I/O was added.
- Only `crates/host-workspace/**` and this task are implemented. The workspace member/dependency lock bootstrap is isolated as `e9b3020` (`build(workspace): register fixed workspace query dependencies`); the earlier unpublished `a6a4de9` was replaced before integration. No other root manifest/lock changes are needed.

## Delivered module

- Explicit immutable canonical workspace binding; factual unconfigured, missing, wrong-type, traversal/symlink and I/O errors. No process-global cwd changes.
- Real files, range read, literal search, Git status, change list and content diff operations. Caller-requested pages/ranges and scoped continuations have no hidden scan/file/result quotas. Git records are never attributed to tools.
- Actual HEAD/index/worktree reads and structured text diffs, including staged/unstaged/untracked/deleted/rename/unborn cases, Unicode fragmentation, and stale-source cursor rejection. Conflict/submodule diff and binary-text limitations are explicit.
- A small per-Session `EditJournal` and serializable retained observations, without automatic capture or storage. Actual write-tool outcome/capture, Session record saving, owner/public Fault mapping and canonical query routing still need integration. The module does not claim Session/TUI end-to-end readiness.
- OS I/O and diff computation run outside future owner state locks; Git uses explicit cwd, does not mutate the index, and has a child-reaping owner. Filesystem/CPU cancellation is cooperative, not OS-call preemption.

## Validation

Environment: Linux, Rust/Cargo 1.90.0. Commands use `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, `PATH=/workspace/shared/cargo/bin:$PATH`, `TMPDIR=/workspace/shared/minicore-host-tmp`, and `CARGO_TARGET_DIR=/workspace/shared/minicore-host-workspace-target`.

- `cargo fmt -p minicore-host-workspace`: passed.
- `cargo test -p minicore-host-workspace --locked`: 20 focused tests passed before the final literal-path/detached/bare fixture was added; final rerun is pending below.
- `cargo clippy -p minicore-host-workspace --all-targets --locked -- -D warnings`: passed before that final fixture/context-bound arithmetic update; final full-workspace check is pending.
- Fixtures use real temporary files, local Git repositories, actual staging/commits/merges and captured file edits. No user repository is mutated by tests. No live provider/model, RPC/TUI, cross-platform or remote-host validation is claimed. No push or main merge occurred.

Full command outcomes and final commit references will be appended after the aggregate gate completes.
