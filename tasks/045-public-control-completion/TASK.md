# Complete canonical public controls

## Scope and fixed source

Base: `76a7bfbe7f70d7f707d48cc38190829d14c137f2`, SPEC v4.93.
Implementation and test source: `570b886fcd4c4315e155d3f244576a681320fa7e`.

- `69385be`: strict protocol requests, metadata/save facts, queue snapshots and
  five additional immutable receipt variants, with protocol tests.
- `24b2b7a`: Agent update, start and default Session controls at actual owner
  acceptance; reuse existing metadata-save ownership and enabled-state logic.
- `8523996`: Session update, input listing and removal at the existing owner
  state boundary. Legacy removal shares the canonical implementation.
- `ee00bea`: six typed client methods and canonical stdio/WSS table routes.
- `c4d3cbe`: real-owner, Runtime, storage and transport acceptance tests.
- `570b886`: async pure validation before canonical update acceptance, with
  single-thread caller deadline regressions. Dropped preparation cannot publish.
  Accepted metadata saves continue under their existing owner task.

No push or main merge. No manifests, lockfile, SPEC, TUI, local-tool producer,
process/shutdown implementation or workspace route changes.

## Public contracts

- Agent/Session update uses `operation_id`, `expected_config_revision` and typed
  `changes`. Agent changes expose name plus AgentConfigPatch. Session changes
  expose clearable title plus SessionConfigPatch. Omission retains current values;
  null clears only supported fields, and arrays replace complete values.
- The direct owner prepares a complete candidate outside its state lock and
  rechecks external authority, lifecycle and configuration version on acceptance.
  Memory updates commit atomically; persistent updates return `status=saving`
  only after the owner has registered the real record-save task. Saving reports
  the still-committed revision, not a successful candidate publication.
- Persistent save completion/failure is read from Agent get or Session state.
  Original operation receipts remain unchanged. Independent invocations sharing
  an operation ID remain separate, so receipt lookup reports genuine ambiguity.
- Agent start only restores Agent scheduling eligibility. It does not clear a
  child pause, execution failure or dependency blocker, or discard old queues.
- Default Session changes require an explicit nullable Session ID and validate
  actual Agent ownership/open state. Saving reports both old committed and
  requested defaults; memory defaults are not persisted as reloadable pointers.
- Input list snapshots the two actual FIFOs, each preserving MessageInput,
  trusted source, actual queue and queue order. It does not list retained bound
  or removed messages, invent a Turn binding, or imply a cross-queue FIFO.
- Input removal uses MessageId and optional expected Controller revision. Its
  lookup/mutation shares the Session lock with Turn binding. Already-bound and
  missing inputs retain their real faults; only the found unbound entry is removed.
- Same-process clients retain external authority. All three adapters call the
  same operations and owners; no JSON round trip is introduced for typed calls.

## Verification

Environment: Rust 1.90.0; locked Runtime `69faf9196991524b5549499d6208a1eaf567bf56`
(version 0.10.0), with no dependency regeneration or upgrades. Commands used
RUSTUP_HOME=/workspace/shared/rustup, CARGO_HOME=/workspace/shared/cargo,
CARGO_BUILD_JOBS=2, dev/test debug=0, CARGO_INCREMENTAL=0,
TMPDIR=/workspace/shared/minicore-host-tmp, and a separate
CARGO_TARGET_DIR=/workspace/shared/minicore-host-public-controls-target.

- Protocol public_controls: 5 tests passed.
- Transport public_controls, public_controls_races, public_controls_storage
  and public_controls_wait: 14 tests passed. Three-adapter cases are distinct
  transport test cases.
- Full `cargo test -p minicore-host-core -p minicore-host-transport --locked`
  at the final source: passed, including its transport doctest.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Workspace doctests: 4 passed. Strict `cargo doc --workspace --no-deps --locked`
  with RUSTDOCFLAGS=-D warnings, formatting and whitespace checks: passed.
- Full `./scripts/check.sh` was attempted twice at preceding fixed `c4d3cbe`.
  The first stopped in unchanged diagnostics file_lease at line 50 with
  LockOutput(WouldBlock). An exact isolated recheck passed. The second passed
  diagnostics, then stopped in unchanged storage test
  complete_final_record_without_newline_is_read_and_append_preserves_prefix
  with HistoryInUse. No diagnostics or storage code was modified here.
  The integration owner is coordinating the independently reproduced lock-lifetime
  issue. Per that owner's direction, a full aggregate is not repeatedly retried
  for an accidental green run; final all-workspace execution remains blocked on
  that separate fix. The 570b886 core/transport regression is independently green.
- `cargo test --workspace --all-targets --locked -- --list`: 669 unique
  target/test-name cases. Nested storage subprocess output is not counted again.
- `cargo metadata --locked --format-version 1`: verifies fixed Git Runtime and
  all 11 workspace crates. No local dependency patch.
- SPEC SHA-256 remains
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

The new cases cover paused and active Turns, same-Turn continuation retaining
actual old model resources, next-Turn use of committed config, new-Session Agent
default inheritance, independent config/message revisions, competing config
updates, stale/read-only external authority, duplicate operation IDs, removal
before/after binding, immutable receipts, actual temp-file save failure, gated
save/default-close races and a lost stdio waiter. Network acceptance uses real
TLS WSS; local independent transport uses actual OS pipes. The resource/model
fixtures are explicitly scripted, not live external models.

Validation logs: `/workspace/shared/public-controls-final-check.log`,
`public-controls-final-list.log`, `public-controls-final-metadata.json`,
`public-controls-core-regression.log`, `public-controls-focused.log`,
`public-controls-validation-regression.log`, `public-controls-validation-clippy.log`
and `public-controls-final-check-rerun.log`.

## Boundaries and next steps

Independent review is coordinated by the parent against the fixed implementation.
Root public coverage documentation is maintained by the integration owner.
`session.context`, persistent delete, application assembly and unrelated workspace
routes are separate stages, not included in this six-method acceptance result.
The existing TUI reference remains `3beeb1d8a525e58ce6352e05061f242c5580e248`,
which locks older Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`.
No TUI, live-provider, production-process or performance validation is claimed.

## Subsequent context route integration

The six-control review boundary remains `570b886`. Independent review passed
that fixed source, including seven separate owner/Runtime/restart/transport
probes, and did not claim the blocked full aggregate passed.

After that boundary, the integration owner authorized these separate commits:

- `bbe5f2d00bd1218cf8180a752f329474fd71fc56`: unchanged cherry-pick of context
  owner implementation `a412d7fbac03c4e810d38ae7b6f34a8be279b0bc`. Host and
  Session module/bootstrap changes merged automatically without conflict.
- `955f07e051a576fdf6f0711bb6193e8a86b58507`: adds SessionClient::context and
  canonical session.context routing using the existing strict SessionTarget.
  Only transport client/operations and transport tests changed in this commit.

The query returns the core SessionContextSnapshot unchanged. Cold persistent
history remains null until actual owner load; known empty history remains an
actual empty snapshot. It does not prepare resources, load a Runner, consume
queued input, fabricate usage, or invoke a model. Read-only clients are allowed.

At fixed combined source `955f07e`:

- 10 context Host integration tests and 2 gated Session save tests passed.
- 4 new typed/OS-pipe/TLS-WSS/cold-persistent transport tests passed.
- All 14 six-control transport acceptance/race/storage/deadline tests passed
  again alongside the context routes.
- Strict all-target workspace Clippy, formatting, whitespace, 4 doctests and
  strict workspace rustdoc passed.
- All-target --list reports **685 unique test cases**. This is a catalog count,
  not a claim that all-workspace execution passed after the known lease failures.
- Locked metadata still has 11 workspace crates and Runtime 69faf919; Cargo.lock,
  SPEC and the TUI are unchanged.

Logs: `/workspace/shared/public-controls-context-owner.log`,
`public-controls-context-regression.log`, `public-controls-context-clippy.log`,
`public-controls-context-list.log`, `public-controls-context-doctests.log`,
`public-controls-context-rustdoc.log` and `public-controls-context-metadata.json`.
The parent coordinates the independent context/route review and combined
aggregate after the separate diagnostics/storage lease fixes. No other topic
was imported and no remote write was made.
