# Select the application readiness diagnostic by component

## Scope and provenance

Base: `985ff19457e682f4d30f6c9bb309a3cbc2d118bf`; topic
`task/084-app-log-fixture`; isolated worktree
`/workspace/shared/minicore-host-task084`.
The requested worker configuration is Astra/max, not independently attested.
The parent owns PROJECT.md, independent review and aggregate integration.

Only the app test event selector, its existing call sites, one real-binary
regression and this task record change. Production, SPEC v4.93, all manifests,
Cargo.lock, Runtime `69faf9196991524b5549499d6208a1eaf567bf56` (0.10.0),
TUI and remote refs remain unchanged. No push.

## Fresh failure and diagnosis

Evidence directory: `/workspace/shared/recovery-second/task084/`.
`env.sh` records Rust's existing shared homes, the task-owned target directory,
two build jobs, zero dev/test debug info and disabled incremental compilation.
Cargo dependency commands use `--locked --offline`.

- On the unchanged base, `cargo test -p minicore-host --test
  credential_environment --locked --offline` failed with exit 101 at
  `tool_support/mod.rs:221`: the selected record had no `listen_port`.
  This reproduces the first Rust test failure from the parent's
  `validation/full-985ff19.log`; it is not evidence of a credential leak.
  See `red-985ff19-credential.log`.
- The added regression launches the actual binary with `env_clear`, one
  synthetic Host key, generated TLS material and Trace logging. Its captured
  stderr records are ordered `host.owner/host.started`, then
  `host.process/host.started`, both with `scope=HOST` and identical actual
  Host identity/incarnation. The first has
  `attributes={"mode":"persistent","phase":"running"}`; the second has
  `attributes={"listen_port":44591,"stdio":true,"wss":true}` in this run.
  The port is dynamically bound, not a fixture constant.
- `red-component-selection.log` retains the complete raw diagnostic lines
  and failed selected-record equality. The red test used the original
  name-only helper; its diagnostic-only stderr printing is not shipped.
  `red-regression.rs` preserves that exact probe, SHA-256
  `70f1709498295011ba968508d4f5518586d0876595d97121f9b4a1cfa1458614`.
- Core `Host::open_configured` emits the owner fact before the application
  starts its transports and emits the process fact. SPEC chapter 12 uses
  component plus event and context to distinguish sources. Neither record
  should be removed, renamed or assigned the other's attributes to fix this
  test. No production correction is indicated by this collision.

## Narrow correction

`Process::event(component, name)` matches both structured fields. All existing
waits request `host.process`: the shared tool-fixture WSS connection, all three
direct WSS readiness consumers in `process.rs`, and the existing process
shutdown/disconnection waits. No text parsing, fallback port, extra sleep,
retry, error suppression or timeout change is added. Captured stderr still
contains all components; the regression checks both startup records and
selects the second without consuming the first as application readiness.

The audit also found a file-output test checking that startup diagnostics
exist without selecting a port. That aggregate presence check remains valid
and unchanged. Other captured lifecycle presence/absence checks also continue
to inspect the complete stream. Credential, TLS/authentication, provider,
Runtime bash, output, history and reopen assertions are unchanged.

## Validation and remaining boundary

- Focused credential and new regression tests pass (`green-focused.log`).
- `cargo test -p minicore-host --all-targets --locked --offline` passes
  all 26 app cases (`app-all-targets.log`), including real binary WSS,
  authentication, ordinary tools, approval and persistence paths. Loopback
  provider replies remain explicit fixtures, not live-provider acceptance.
- `cargo fmt --all -- --check` passes (`fmt.log`).
- `cargo clippy -p minicore-host --all-targets --locked --offline -- -D warnings`
  passes (`app-clippy.log`).
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host --no-deps --locked
  --offline` passes (`app-doc.log`). The app has no library doctest target.
- The scoped `cargo test -p minicore-host --all-targets --locked --offline --
  --list` confirms 26 unique app cases (`app-test-list.log`). Filtered offline
  Cargo metadata selects only the pinned Runtime, version 0.10.0
  (`metadata.json`, `runtime-and-count.txt`). This is not a workspace test count.
- Whitespace, protected-source and unchanged credential-test checks pass.
  SPEC SHA-256 remains
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`;
  Cargo.lock SHA-256 remains
  `1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`.

Independent review remains pending. The workspace-wide final integration gate, live-provider/TUI
acceptance and performance acceptance are not claimed by this fixture task.
