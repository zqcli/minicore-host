# Task 044: Route workspace and change queries

## Scope and source

Implement SPEC v4.93 §§8–10 canonical `workspace.files`, `workspace.read`,
`workspace.search`, `workspace.status`, `changes.list` and `changes.diff`.
Reuse host-workspace requests/results and actual owners; do not add a second
backend, mutation API, durable edit database, UI state, capacity quota or retry.

Worktree: `/workspace/shared/minicore-host-workspace-routing`, topic
`feat/workspace-query-routing`, initially fixed
`47e6b04382c1eddf67ca27f5a310fb8491441111`. The minicore-agent checkout is code
reference only. TUI, main, root dev and remotes are unchanged. The requested
Astra/max setting is not separately runtime-attested.

Prerequisite integration preserves actual tool ownership:

- Core workspace dependency `e8cceab` -> `1e75280`; routing module/dependency
  bootstrap `90dfd7a` -> `ec5b703`.
- Production tool bootstrap `f1a33d5` -> `ba83f5c`, original output/cleanup lease
  `7f7c280` -> `ac0a90b`, Session edits `6fcc85a` -> `f3feba4`, fresh/reopened
  coverage `1045aa4` -> `2330602`.
- Tool recording `70f731d` -> `7da036f`, production resources `10fd934` ->
  `6a62dbd`, binary fault wording `1bb488b` -> `106d90e`, production tests
  `6b4bef5` -> `d3e93ce`.
- The first direct cherry-pick of `6fcc85a` was stopped by missing lease
  prerequisites. The parent aborted that incomplete pick, integrated the chain
  in order, retained compaction exports and workspace/tool_edits modules, and
  verified the worker's four unstaged files by SHA before restoring them.

## Implemented contract

- Host resolves the explicit Agent -> Session, clones that Session's committed
  configuration under its lock, then binds and reads outside every owner lock.
  An active Turn's frozen workspace is never used to select current queries.
- Results reuse `SessionIdentity` and the original backend payload in `data`.
  Filesystem/Git results identify the captured config revision. Retained edits
  have no fabricated current revision, Load ID or old Turn identity.
- Workspace changes are actual current Git facts with `workspace_unknown`
  origin. Session/Turn changes come only from the actual Session EditJournal;
  diffs clone its original record before I/O and use exact before/after bytes.
- Missing/reclaimed observations are explicit Faults. Unload preserves existing
  observations. Persistent reopen does not claim complete Session edit coverage;
  current-incarnation retained Turns are independently queryable.
- Typed, stdio and WSS use the same operations table and owner methods. Read-only
  clients are accepted. CallOptions remains only a client wait budget. Query
  timeout/disconnect and optional embedding QueryContext do not cancel a Turn.
- Public leaves 3801–3812 classify the backend's existing failures without new
  hierarchy nodes, details or retry hints. Messages exclude raw I/O/Git stderr.
  The authorized narrow Session hook mapping uses the new range/cursor/change
  leaves; recording and retention ownership are unchanged.

## Validation in progress

Environment used for commands:

```
RUSTUP_HOME=/workspace/shared/rustup
CARGO_HOME=/workspace/shared/cargo
PATH=/workspace/shared/cargo/bin:$PATH
TMPDIR=/workspace/shared/minicore-host-tmp
CARGO_BUILD_JOBS=2
CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_TEST_DEBUG=0
CARGO_INCREMENTAL=0
CARGO_TARGET_DIR=/workspace/shared/minicore-host-workspace-routing-target
```

- Initial focused compilation failed because a test helper used `&Host` instead
  of the existing `&Arc<Host>` asynchronous creation receiver. Corrected.
- First six focused cases passed. A repetition exposed an overly specific
  reclaimed-source test expectation: the sole Session's memory Agent can also
  retire. The test now accepts either actual missing direct owner, not success.
- `cargo test -p minicore-host-transport --test workspace_queries --test
  workspace_tool_changes --test workspace_wait --locked` passed all 8 cases
  before the final fault-mapping unit test was added. Each relevant case uses
  typed plus real Unix pipe stdio plus certificate-verified TLS WSS. Log:
  `/workspace/shared/workspace-routing-focused-all2.log`.
- These tests use real Host, Runtime, production resource builder, filesystem,
  Git and write/edit tools, with explicitly local scripted HTTP/SSE responses.
  They do not claim a live provider, production CLI process or TUI validation.
- The timeout test deterministically occupies the test runtime's sole blocking
  worker. It verifies query waits expire while owner updates remain available,
  original query config/root stay captured, and the actual gated Runtime Turn
  completes after observer disconnection.

Final exact source hash, full gate, inventory and repeated results follow after
source freeze. Independent review and parent aggregate integration remain
separate gates; no publication is authorized by this task record.
