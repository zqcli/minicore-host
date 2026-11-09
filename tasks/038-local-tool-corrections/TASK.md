# Task 038: Correct local-tool ownership

## Scope

Frozen starting source: `3cdd977c70e559286a72cf06e485a5e8a89b4261`.
Fix the two independently reproduced P1 local-tool defects: mutation ownership
shared by Workspace clones, and actual Unix process-group completion evidence.
Runtime stays at `69faf9196991524b5549499d6208a1eaf567bf56` / 0.10.0. SPEC v4.93,
TUI, Host core, registry, resources, transport and protocol are unchanged.

The Agent checkout `f36a87366d8e2fe0a1c8216463887902e471c739` is a code reference,
not a behavior or compatibility baseline. Exact-text read/edit pairing, JSON
business results, Runtime-aligned deadline, full byte-exact output, public-Turn
Scope and LocalToolSink remain the Host interfaces. No file/output quota,
automatic write/command retry, old LoopId owner, or cancel-on-waiter-drop is added.

The coordinator supplied dependency commit
`708a18ff27bb16a7b58b723a15986b4d4046c4b1`, cherry-picked here as `77a5190`:
Unix-only `process-wrap =8.2.2`, default features disabled, with only `tokio1`,
`process-group`, and `kill-on-drop`. The lockfile changes add its six missing
packages without updating any previously locked version or Runtime revision.

## Corrected boundaries

- Workspace owns one dedicated mutation-operation gate shared by its clones.
  LocalTools transfers the owned guard into the actual blocking file worker,
  covering source capture, replacement, authoritative after capture and the
  short owner callback. This is not an owner-state lock across I/O.
- All LocalTools constructed from clones of that binding coordinate. Separate
  `Workspace::open` / `Workspace::bind` calls, even for equal canonical paths,
  do not share the gate. Shell writes and external processes are not coordinated.
  No Host-global path-lock service, filesystem isolation, or cross-process lock
  is introduced.
- Bash still stops remaining original-group members after natural leader exit,
  cancellation or deadline. A successful signal request is not completion.
  One full process-wrapper wait is joined without cancellation, concurrently
  with stdout and stderr draining. A separate `killpg(..., None)` probe must
  return ESRCH before cleanup can be confirmed. Other errors remain unknown;
  a group still present after ten 20 ms observation intervals is unconfirmed.
- Direct-child exit status, group-end evidence, real pipe EOF and cleanup are
  distinct facts. A failed wrapper wait cannot confirm cleanup. Real output is
  preserved; a pipe not reaching EOF during the existing one-second cleanup
  allowance stays incomplete. No detached reader or falsely successful finish
  is substituted for waiting.

## Reference reuse and Host differences

Adapted from Agent `src/tools/command.rs`: wrapped ProcessGroup spawn and
KillOnDrop backstop (755–772), one non-abandoned full wrapper wait (872–888),
concurrent wait/drain sequencing and remaining-group cleanup (489–708), and
ESRCH-only group observation with unknown distinct from gone (1115–1143).
The old comment treating a remaining same-group member as outside a sandbox was
not copied. The Host keeps immediate group SIGKILL and its existing cleanup
budget rather than importing unrelated TERM grace, tail caps, Windows jobs,
ToolOwners quotas or old Session/Loop authority.

Two real-process regressions adapt Agent command tests at 547 and 683: redirected
background members must end before Turn finish, and cancellation joins both
leader and group member. Both require original-group ESRCH, actual child waits,
real stream EOF and one side effect.

Agent `src/workspace/atomic.rs` was inspected for same-directory temporary-file
commit, permission preservation and commit/observation ownership. Those useful
mechanics already exist here. Its `expected_matches` check followed by rename is
not atomic conflict coordination, so it was not imported as a purported fix.
The small Host mutation capability is new. Old schemas/defaults, CRLF rendering,
quotas, and prose were not restored.

## Verification evidence

Evidence directory: `/workspace/shared/minicore-host-local-tools-fix-evidence`.
Fixed external probes are copied to
`/workspace/shared/minicore-host-local-tools-fix-probes`; only their manifest paths
and dependency lock resolution differ from the original review package. The
probe assertions and source bytes are unchanged:

- `tests/probes.rs`: SHA-256
  `b26ea7dbc951cf3b3c67398ca41abc07a2b2f7d10e9e98af6d300ed5e701fc51`.
- `tests/group_evidence.rs`: SHA-256
  `2168a1ef4b987940ae266acb122377728a519c1f449455ac6a456d080d1e559a`.

The original 26 local-tools tests passed again on frozen source. The fixed
12-case external suite failed 3 cases on that same source: 5/12 disjoint-edit
losses, 5/12 attribution anomalies, and 4/8 live-group confirmations. No test was
weakened. The new redirected-background native regression also failed before the
command fix because the allegedly completed group still returned `Ok(())`
instead of ESRCH.

Source commits:

- `80945814207f5e687de26d2c6a05d725dda9a850`: process-group wait and positive
  group-end confirmation, with the two real-process regressions.
- `c8703636a8606810c05c877ac8f522a6091e51ba`: Workspace-owned mutation guard and
  the deterministic clone-binding regression.
- `7778869d28854c0274de991a2d95ee15832a5677`: test-only readiness correction;
  cancellation tests wait for real nonempty bytes instead of file creation.

The mutation regression was separately run red in an external source snapshot
with only the old per-LocalTools gate/lifetime restored. The explicitly held
Workspace gate failed to block those calls, as expected. This confirms the small
regression without relying on large-file timing. That snapshot initially reused
the fix build target, causing one subsequent green invocation to execute its
stale red test artifact. The stale run is superseded, not a production failure:
the live source was unchanged, then the two owned packages were explicitly
cleaned and rebuilt from this checkout before final focused/full validation.
Red snapshots must use a separate target in future. No active other cache was
deleted, and no assertions were weakened.

Existing Runtime tests use explicitly scripted model/HTTP fixtures; real
Runtime, filesystem and Unix processes are executed. No live model provider or
TUI validation is claimed.

Environment: RUSTUP_HOME=/workspace/shared/rustup, CARGO_HOME=/workspace/shared/cargo,
PATH=/workspace/shared/cargo/bin:$PATH, TMPDIR=/workspace/shared/minicore-host-tmp,
CARGO_BUILD_JOBS=2, CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0,
CARGO_INCREMENTAL=0,
CARGO_TARGET_DIR=/workspace/shared/minicore-host-local-tools-fix-target.

## Remaining production integration

This remains a standalone local execution module. Current production resource
preparation (`crates/host-resources/src/lib.rs`, `prepare_turn`) calls
`Workspace::bind` for each Turn. Integration must retain/reuse the selected
Workspace binding across owners whose file mutations should coordinate, and
clone it into their Turn-local tools. Equal path strings alone do not grant that
coordination. Existing immutable Turn/resource behavior must remain intact.

Production registry selection, Session persistence, public output/change
projections and actual finalization integration remain separate work. The owner
must retain LocalTools across all internal loops of one public Turn and wait for
`finish()`. A false cleanup result must remain blocked finalization, not a
published finished state. This task does not change Runtime or claim general
process supervision, escaped-group cleanup, hostile filesystem protection,
Windows job support, or a filesystem sandbox. No push is performed.

## Final verification

Final tested source: `7778869d28854c0274de991a2d95ee15832a5677`; the following
record update changes documentation only. All commands use the environment
above unless the target override is shown.

Red reproduction commands:

- In the frozen review checkout: `cargo test -p minicore-host-local-tools
  --all-targets --locked -- --nocapture` passed all original 26 cases using
  `CARGO_TARGET_DIR=/workspace/shared/minicore-host-local-tools-review-target`.
- In the original review probe package: `cargo test --offline --locked
  --test probes -- --nocapture` failed exactly the 3 known cases (9 passed).
  Log: `baseline-independent-probes.log`.
- Before the command fix: `cargo test -p minicore-host-local-tools --locked
  --test process redirected_background_members_end_before_turn_finish --
  --nocapture` failed the positive group-end assertion. Log: `background-red.log`.
- In `/workspace/shared/minicore-host-mutation-red.dup0ps`: `cargo test
  -p minicore-host-local-tools --test files --locked
  separate_turn_tools_share_mutation_order_and_disjoint_edit_snapshots --
  --exact --nocapture` failed the held-gate assertion. Log: `mutation-red.log`.

Final green commands:

- `cargo clean -p minicore-host-local-tools -p minicore-host-workspace`, then
  `cargo test -p minicore-host-local-tools --all-targets --locked -- --nocapture`:
  **29 passed** (all original 26 plus 3 regressions). Only this task's build
  target and these owned packages were cleaned. Logs: `clean-owned-packages.log`,
  `final-local-tools.log`.
- `./scripts/check.sh`: passed on the final tested source, including whitespace,
  fmt, strict workspace Clippy, all-target tests, doc tests and warnings-denied
  rustdoc. Log: `final-head-gate.log`.
- `cargo test --workspace --all-targets --locked -- --list`: **523 unique test
  cases**, plus **3 doc tests** passed by the gate. Nested storage subprocess
  output is not counted again. Log/inventory: `final-head-test-list.log`,
  `final-test-inventory.json`.
- `cargo metadata --locked --format-version 1`: confirms actual Runtime 0.10.0
  at `69faf9196991524b5549499d6208a1eaf567bf56`, no patch override, process-wrap
  8.2.2. No previously locked dependency version changed. The downloaded official
  process-wrap archive SHA-256 matches Cargo.lock. File: `final-metadata.json`.
- In the copied fixed probe package: `cargo test --offline --locked --test
  probes -- --nocapture`: **12 passed**, including real Runtime cancel/deadline,
  natural finish and late cancellation, no-repeat effects, abandoned execute
  and finish waiters, output/edit/final-command sink failures, byte-exact large
  output, and both cross-binding file cases. Log: `final-independent-probes.log`.
- `cargo test --offline --locked --test group_evidence -- --nocapture`:
  **1 passed**, 8 rounds, **0** premature original-group cleanup confirmations.
  Log: `final-group-evidence.log`.
- `cargo test --offline --locked --test probes independent_ -- --nocapture
  --test-threads=1`: **2 passed**, 12 rounds each, **0** lost disjoint edits and
  **0** write-attribution anomalies. Log: `final-isolated-file-probes.log`.

The known, separately tracked task024 subprocess-fixture issue did not reproduce
in these full gates. No fixture or locking code for that topic was changed, and
its correctness is not claimed by this task. The standalone local-tool branch
remains unpublished; production multi-Session integration remains open as stated
above, not inferred from these probe results.
