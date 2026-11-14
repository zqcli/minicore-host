# MiniCore Host workspace and directory operations

A small backend module for SPEC v4.10.3 §§2, 7–9. The six workspace/change reads use actual files, Git objects/index/worktree state, and explicitly retained tool observations. There is no model call, process-global `chdir`, database, cache owner, resource quota, or compatibility layer.

## Host directory selection

`WorkspaceDirectories::prepare` canonicalizes only explicitly configured absolute
roots during Host resource preparation. `roots/list/mkdir` operate on that immutable
selection without requiring a Session. List results contain direct directories only,
including symlinks whose targets remain inside an allowed root. They use absolute
Host-platform paths, preserve Windows drive/UNC syntax, and reject unrepresentable
UTF-8 paths rather than inventing lossy names. Names use the Host platform's rules.
List cursors bind all prepared roots, the canonical directory and hidden filter;
page size is caller-selected with no fixed entry cap. These are live observations.

Host owns authentication, operation rights, blocking-worker lifetime, shutdown
settlement, error mapping and the mkdir receipt. `mkdir` creates one actual child,
never recursively creates parents or overwrites an existing entry. The same local
path containment limits described below apply; this is not isolation from a hostile
same-user process changing filesystem components concurrently.

## Owner integration

Resolve `Workspace::bind(config.workspace.as_ref())` outside the Session lock. A root must be an absolute existing directory. Keep the resulting immutable binding with the configuration/Turn that selected it. `None`, missing directories, and invalid bindings fail explicitly; nothing falls back to the Host's cwd. Clone a binding under the owner lock, then release that lock before calling its async methods.

The crate supplies typed query data, not a canonical `SessionService`, RPC router, authorization decision, or public Fault registry. The Session layer still owns target/ownership checks and mapping `WorkspaceError` to its registered Faults. No TUI or live model integration is claimed here.

| Public operation | Real source and result |
| --- | --- |
| `Workspace::files` | Sorted relative directory/file/symlink entries, optional path substring and explicit page size |
| `Workspace::read` | Actual regular-file bytes in the caller's half-open UTF-8 byte range |
| `Workspace::search` | Literal text matches, case mode, file/directory scope, matching lines and UTF-8 byte spans |
| `Workspace::status` | Git porcelain v2, branch/HEAD, staged/worktree codes, rename source, conflict and untracked entries |
| `Workspace::changes_list` / `EditJournal::list` | Current Git records or separately retained Session/Turn tool records |
| `Workspace::changes_diff` / `diff_edit` | Actual HEAD/index/worktree content or the tool's retained before/after bytes, as structured text hunks |

Requests specify their page sizes and ranges. There is no application maximum or hidden scan/file/body budget. A zero page size or a text page unable to fit the next UTF-8 character is invalid. `QueryContext` carries the caller's cancellation and optional absolute deadline, with no default deadline. Filesystem and diff work run on blocking workers. Cancellation is cooperative between I/O chunks, entries and lines; a synchronous OS call or an in-progress diff computation is not preempted. Similar receives the caller's deadline and the result is checked afterward. Each Git subprocess has an owned task that drains its output and reaps it, including after cancellation or a dropped waiter.

## Files and continuations

- Paths are workspace-relative, with `/` separators in returned paths on every platform. Absolute paths, `..`, NUL and resolved symlink escapes are rejected. Windows also rejects alternate streams and trailing-dot/space aliases. Directory traversal never follows directory symlinks or junctions. Direct reads may follow a symlink only inside the binding. Special files are not read as regular files.
- Files/search use the `ignore` traversal's rules encountered within the requested subtree, including `.gitignore`, `.ignore`, and Git exclusions. Parent/global ignore files are disabled. Hidden files are included; `.git` entries are not traversed. An explicitly selected file is read/searched directly. Binary/non-UTF-8 search files are named in `skipped_binary_files`; non-UTF-8 paths and actual traversal/read failures are errors.
- `files` is ordered by relative path. Its cursor binds the workspace, directory, recursion and path filter. `search` is ordered by path then one-based matching line. All matches on a line are returned; its cursor binds the workspace, selected path, literal and case mode. Page size can change. These are live observations, not frozen filesystem snapshots. Insertions/edits behind a continuation are not replayed.
- `read.start/end` and returned positions are UTF-8 byte offsets. `end: None` selects the current end; an explicit end past EOF is clamped. Returned text is only the requested byte range, with the last complete character that fits `max_bytes`. Continue at the returned `end`. `has_more` refers only to the requested range. NUL/invalid UTF-8 in that range is a binary error. A detected size/mtime change during a read is an error, not an empty success.
- The containment checks cover ordinary local traversal/symlink escapes. They are not a sandbox against a hostile process changing paths concurrently as the same OS user.

## Git and diffs

Git is a required executable for Git queries. Status is read-only (`GIT_OPTIONAL_LOCKS=0`); fsmonitor execution, lazy fetching, terminal prompts and inherited Git directory/index/discovery overrides are disabled. The `GIT_CONFIG` override is also cleared because it redirects `git config` alone, which would make a filter preflight inspect different settings from status. No external diff/textconv or repository hooks are invoked. Git commands use an explicit cwd and literal pathspecs. Queries support nested workspaces and linked worktrees. A plain directory/bare repository has `repo_available: false`; `changes_list` there returns `NotRepository`. Missing Git, a corrupt repository or a missing binding is an error, not a clean status.

Before status (and the status refresh used by change queries), a private read-only preflight matches effective nonempty `filter.*.clean` / `filter.*.process` commands to the actual tracked paths' filter attributes. With stable configuration and attributes, a matching external driver returns `UnsupportedFilter` before status starts; it is not silently disabled. Configured but unused drivers, including global Git LFS-style definitions, do not block ordinary queries. The same check covers initialized gitlink worktrees that Git status will inspect; it does not recursively scan arbitrary directories or uninitialized modules. A gitlink replaced by a directory symlink is explicitly unsupported before its target is inspected. Under that stable-input assumption, the rejection prevents those filter descendants from retaining query pipes or performing effects after caller cancellation. Stdin writing and output draining remain inside the owned Git task.

The filter preflight assumes Git configuration and attributes stay stable for the duration of the query. It is not an atomic snapshot or a cross-process lock: uncoordinated concurrent changes can make status execute a filter after preflight, including changes to configuration shared with another worktree. This first version does not isolate queries from that external race. Another worktree's own attributes do not affect this worktree, but its changes to shared Git configuration can.

A failed repository-discovery call with a gitfile/symlink or nonempty `.git` directory in its ancestry is an error. Completely empty `.git` directories contain no repository state and may be ordinary placeholders; they do not establish a repository by themselves.

Git changes always have `origin: workspace_unknown`. They are never labeled as tool edits. Rename detection and cross-directory scoping follow Git's actual status behavior. Duplicate status entries for a single path remain distinct and survive pagination. Status/list and diff are separate live observations: diff refreshes the sources and marks `stale` when its list-time status/HEAD changed. Diff cursors fingerprint the exact compared bytes, comparison, context and binding and reject changed sources rather than combining different pages.

Diffs compare file content, including link text for a Git symlink without following its target. `HeadToIndex`, `IndexToWorktree`, and `HeadToWorktree` are explicit. Unborn HEAD is a real missing base. Missing files and present empty files are distinct snapshot values. Text hunks preserve LF, CRLF, and missing final newlines; long lines continue at UTF-8 byte offsets. Hunk coordinates refer to the complete logical hunk even when a page carries fragments. Binary content is explicitly marked without invented text hunks. Text hunks do not represent executable-bit-only changes. Conflicts and submodule content comparisons are explicitly unsupported rather than guessed.

## Required tool-edit handoff

`changes.rs` implements current Git/workspace observations and shared change
values. `edits.rs` holds caller-owned tool evidence; `diff.rs` shares the content
comparison implementation. Public crate exports are unchanged.

`EditJournal` is a plain append-only value owned by one Session, with no I/O, locks, tasks or implicit saving. `EditRecord` retains the actual `ToolCallRef`, path, before/after snapshots and observed `Applied` / `NotApplied` / `Unknown` outcome. Each stored position plus the bound Session/ToolCall identifies a record; repeated edits of one file remain separate observations.

The integrated write/edit tool owner in `host-local-tools` and its Session sink
follow this handoff:

1. Capture the actual before state (or explicitly retain `Unknown` if capture failed).
2. Carry the real write and its completion/uncertain outcome under that tool's existing owner. Dropping a caller must not drop this observation.
3. Append the actual after observation and outcome once, without turning intended bytes into proof of a completed write.
4. Retain observations under the actual Session owner. Native ToolResults use its normal history save boundary; edit snapshots themselves are not independently persisted. `records()` is serializable and `from_records` reconstructs the same order, but this is not an automatic persistence or recovery service.

For a diff, use `EditJournal::get` while accessing Session state, clone the returned `Arc<EditRecord>`, release the lock, then call `diff_edit` with the same validated reference. Unknown snapshots fail explicitly; they are never treated as empty files. Capturing or listing current Git state cannot reconstruct missing historical tool observations. Session capture and public Fault mapping are integrated, and `host-transport` exposes the six queries on typed/stdio/WSS connections. `host-transport/tests/workspace_tool_changes.rs` covers real tool edits and missing old coverage after reopen. Actual TUI integration remains unverified.

## Verification

`cargo test -p minicore-host-workspace --locked` uses isolated temporary directories and Git repositories. It does not mutate a user's checkout, contact a remote, invoke a model, or claim TUI coverage. The parent workspace's `scripts/check.sh` runs formatting, strict Clippy, all targets, doctests, rustdoc and whitespace checks. Exact results and scope are recorded in `tasks/014-workspace-queries/TASK.md`.
