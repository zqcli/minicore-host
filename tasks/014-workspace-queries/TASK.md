# Implement workspace queries

Status: module implementation and full workspace validation complete; independent review and canonical Session/tool integration are separate stages.

## Scope and provenance

- SPEC v4.93 §§2, 7–9, 11 and 13; baseline `413916cac0730d291e5c8163cdc3d59e5dbedcd1`.
- Reference implementation read at `minicore-agent:f36a87366d8e2fe0a1c8216463887902e471c739`: `src/workspace*`, `src/changes.rs`, `src/diff.rs`, and `src/tools/file_change.rs`. TUI `3beeb1d8a525e58ce6352e05061f242c5580e248` workspace/change DTOs informed path/kind, Git XY, hunk and line fields. TUI is unchanged.
- Runtime stays locked at `18a6ac9b82d003330cd870275efca793cdcc9a96`. This crate needs no Runtime dependency or public protocol changes. No version negotiation, storage migration, defensive recovery framework, application capacity quota or owner-lock I/O was added.
- Only `crates/host-workspace/**` and this task are implemented. The workspace member/dependency lock bootstrap is isolated as `e9b3020` (`build(workspace): register fixed workspace query dependencies`). No other root manifest/lock changes are needed.

## Delivered module

- Explicit immutable canonical workspace binding; factual unconfigured, missing, wrong-type, traversal/symlink and I/O errors. No process-global cwd changes.
- Real files, range read, literal search, Git status, change list and content diff operations. Caller-requested pages/ranges and scoped continuations have no hidden scan/file/result quotas. Git records are never attributed to tools.
- Actual HEAD/index/worktree reads and structured text diffs, including staged/unstaged/untracked/deleted/rename/unborn cases, Unicode fragmentation, and stale-source cursor rejection. Conflict/submodule diff and binary-text limitations are explicit.
- A small per-Session `EditJournal` and serializable retained observations, without automatic capture or storage. Actual write-tool outcome/capture, Session record saving, owner/public Fault mapping and canonical query routing still need integration. The module does not claim Session/TUI end-to-end readiness.
- OS I/O and diff computation run outside future owner state locks; Git uses explicit cwd, does not mutate the index, and has a child-reaping owner. Filesystem/CPU cancellation is cooperative, not OS-call preemption.

## Validation

Environment: Linux, Rust/Cargo 1.90.0, Git 2.52.0. Commands use `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, `PATH=/workspace/shared/cargo/bin:$PATH`, `TMPDIR=/workspace/shared/minicore-host-tmp`, and `CARGO_TARGET_DIR=/workspace/shared/minicore-host-workspace-target`.

- `cargo fmt -p minicore-host-workspace`: passed.
- `cargo test -p minicore-host-workspace --locked`: 21/21 tests passed (8 filesystem, 8 Git, 5 edit journal). This crate currently has no doctest examples.
- `cargo clippy -p minicore-host-workspace --all-targets --locked -- -D warnings`: passed; the final whole-workspace Clippy rerun also passed.
- `./scripts/check.sh`: exit 0. `check-whitespace.sh`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --all-targets --locked` (261 tests: 42 lib + 219 integration), `cargo test --workspace --doc --locked` (2 doctests), and `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` all passed on the final source/test bytes.
- `cargo test --workspace --all-targets --locked -- --list` independently confirmed 261 unique tests: the 240-test baseline plus this module's 0 lib / 21 integration tests. A storage test launches a filtered child test process whose extra `1 passed; 36 filtered out` output must not be counted again.
- The fixed source and tests are `3194521` (`feat(workspace): read bound files and real change sources`) and `e40f5ef` (`test(workspace): verify real files git states and edit snapshots`). A caller requesting all available diff context is clamped only to the actual line count, avoiding arithmetic overflow without an application quota.
- Fixtures use real temporary files, local Git repositories, actual staging/commits/merges and captured file edits. No user repository is mutated by tests. No live provider/model, RPC/TUI, cross-platform or remote-host validation is claimed. No push or main merge occurred.

## Integration boundary

Apply original commits `e9b3020`, `3194521`, and `e40f5ef` once, then this validation-note commit. The sole root manifest/lock change is the bootstrap; all implementation and test paths stay inside the assigned crate/task. Independent review may proceed at `e40f5ef` while these notes are finalized.

Before declaring the six canonical methods delivered, the Session service must select/authorize the actual binding, map the typed local errors into the public Fault registry, route workspace versus Session/Turn scopes to their real source, and connect the existing tool write owner to retained/saved edit observations. Unknown prior tool snapshots are not recoverable by scanning Git. Tests establish the reusable real module, not those remaining integrations.


## Parent integration checkpoint

Topic-to-dev mapping: `e9b3020` → `a48201f`, `3194521` → `d27575d`,
`e40f5ef` → `b0fdbfb`. The following topic `32686dc` only records final validation.
At fixed `b0fdbfb`, the main workspace `./scripts/check.sh` passed all 275 unique
unit/integration tests and 2 doctests, formatting, strict Clippy/rustdoc and
committed whitespace. The 21 module cases add to the 254-test canonical-control
baseline. `cargo test --workspace --all-targets --locked -- --list` independently
confirmed 275 entries; nested storage process probes were not counted again.
Logs: `minicore-host-validation/workspace-integrated-b0fdbfb.log` and
`workspace-test-list-b0fdbfb.txt`. Independent scoped review is in progress.
No canonical Session integration, real write-tool recording or push is claimed.
## Review follow-up: repository discovery

Independent review reproduced a missing `.git/HEAD` being reported as a plain directory. Failed Git discovery now checks for a gitfile/symlink or nonempty `.git` directory in the bound directory or its ancestors before returning `repo_available: false`; existing damaged markers preserve a real repository-discovery error. Inherited discovery ceilings/overrides are cleared alongside Git directory/index overrides so a selected repository subdirectory is not incorrectly hidden. Real temporary fixtures cover missing HEAD at root and nested bindings, an incomplete `.git` directory retaining its config, a broken gitfile and a child-process-only inherited ceiling. Completely empty `.git` directories contain no repository state and are not treated as initialized repositories. The genuine plain-directory fixtures use Linux `/tmp`; no user process environment is mutated by the tests.


## Review follow-up: external Git filters

Independent review demonstrated that read-only Git status invokes active clean/process filters, including filters configured only inside initialized submodules. A slow filter also survived direct Git cancellation while retaining output pipes. The fix uses one private preflight over effective driver definitions, scoped tracked paths and their actual filter attributes, including initialized gitlinks. Queries with executable filter bindings return `UnsupportedFilter` before status starts. Filters are not disabled, and unused global driver definitions do not block a query. Gitlink directory symlinks are rejected before following their targets; uninitialized gitlink directories are not traversed.

`check-attr` stdin writing and all output drain tasks remain inside the existing owned Git task and are joined before an error/cancellation is propagated. Inherited `GIT_CONFIG` is cleared because it redirects the config-only preflight without equally redirecting status. This is a local Git query safeguard, not an execution policy, process supervision platform, application quota or public protocol change. Real fixtures cover active clean/process commands, query list/diff propagation, no script launch with cancellation, child-local submodule drivers, unused global configuration, a config-only override, scoped attributes and a symlinked gitlink. Existing 21 tests are retained.
