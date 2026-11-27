# Task 126: Host-backed editor file references

- Baseline: `929c6632c2c614d02f3ddae455f672a5bf096bc5` (`dev`).
- Implementation checkpoint: `2ebf74cbdcc2249fef5ab97a038a259f5f077836`.
- Published implementation: [`8930d26bc4d749a2b214d820dd84d9db13554c7d`](https://github.com/zqcli/minicore-host/commit/8930d26bc4d749a2b214d820dd84d9db13554c7d), included in [Host 0.12.18](../../PROJECT.md#current-checkpoint-host-01218).
- Scope: additive file browsing through `workspace.list`; package version stays
  0.12.17 at the original checkpoint and SPEC advances to 4.11.13. No new RPC,
  content read or execution.
- Worker requested as `gpt-6-astra` / `max`; no claim of independently inspected
  runtime settings.

## Contract and implementation

- `DirectoryListRequest.include_files` defaults to false. Opting in lists
  directories and regular files; `is_file` distinguishes them. A direct absolute
  file path resolves to canonical metadata with no entries or continuation.
- `workspace_session` uses normal Host -> Agent -> Session queries to add the
  selected Session's configured workspace for this read only. Explicit Host
  `workspace_roots` remain available; this does not change global roots or mkdir.
- Both listing and direct file resolution retain canonical-path authorization.
  Escaping symlinks are excluded/refused. Parent navigation uses only the
  authorized `result.parent`; no parent is returned at an allowed root boundary.
- The old directory-only JSON shape omits all new default-valued fields. Paging,
  hidden filtering, and bounded page selection reuse existing code. Cursors
  also bind the include-files mode. No new dependency or model/tool capability.
- Implementation: `host-protocol/src/directories.rs`,
  `host-workspace/src/directories.rs`, `host-core/src/host/directories.rs`.

## Verification and handoff

Pinned Rust 1.90.0 with locked dependencies, on the implementation checkpoint:

- `cargo fmt --all -- --check` and `./scripts/check-whitespace.sh`: passed.
- `cargo test --locked -p minicore-host-protocol --test directories -- --quiet`:
  2 passed, including default wire omission/round-trip.
- `cargo test --locked -p minicore-host-workspace directories::tests -- --quiet`:
  10 passed, covering direct files, file symlinks and paging/filter scope.
- `cargo test --locked -p minicore-host-transport --test directory_picker -- --quiet`:
  3 passed. The new test exercises real Host queries over in-process, stdio
  and loopback WSS with empty or configured additional roots. It also checks
  read-local Session scope, parent boundaries and unchanged Host roots.
- `cargo clippy --locked -p minicore-host-protocol -p minicore-host-workspace
  -p minicore-host-core -p minicore-host-transport --lib -- -D warnings`: passed.

Implementation complete; focused Linux verification passed. No model requests
were made by these fixtures. For the published implementation and cross-platform
Host CI, see [PROJECT.md](../../PROJECT.md#current-checkpoint-host-01218). Manual
TUI interaction and independent review are separate from these checks.
