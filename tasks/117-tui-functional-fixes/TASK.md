# Task 117: TUI query paste, forward delete and Session identity

## Goal, scope and current status

On 2026-11-15 the owner approved exactly three repairs, required the specification
before implementation, requested `gpt-6-astra` developers with `max` reasoning
and no Codex CLI, and requested a 0.0.1 version increment. Previous UI-01/current
item 2 and previous UI-03/current item 1 are two repeated reports of two scopes;
the confirmed long-title Session-picker ambiguity is the third. Task 117 was
unused when this work began.

Implementation and independent source review are complete. Host/TUI 0.12.4 are
published on `dev`; the complete local gates and the fresh native-terminal
sequences are complete. **All nine Host and eight TUI CI jobs succeeded on the
exact published release commits**, and both workflow conclusions are completed
success. This task closes the three approved repairs and their release gates.

## Baselines and specification-first publication

- Host baseline: `aeb09650d01a32c98a7266e8e19f92efebbb5be4`, version 0.12.3,
  including Task 116's completed Windows fixture/documentation follow-up.
- TUI baseline: `0b7d4dc799fbe8c124661713bf8300fc3686ca78`, version 0.12.3,
  embedding Host `dbaaa8175780759b84d26e0519aa4b27ced5f8f4`.
- SPEC moves from 4.11.2 to 4.11.3. Final SHA-256:
  `21108befba6187fcc1355f9b11caae597c3d3f4b6611b5cea7e712c1f6e8b735`.
- Host and TUI are both 0.12.4; native object/history formats remain schema 3.

| Checkpoint | Local commit | Canonical published commit | Identical Git tree |
| --- | --- | --- | --- |
| Host specification | `a108614ffec569f4b7e9875c12b28920cb649909` | `421b3d9bc05c166bc6f546390d9a987dbf771874` | `09d70c596a38cce4e0e6253e68c2dedba575f72b` |
| Host patch release | `93178950d248edf34105495c367353895cca395a` | `667f0923b827fea9b5fdb60f75a21f5369e44193` | `8c40594c4b97331e4864ee1b749d8753e107fa2a` |
| TUI design contract | `f4fbee0d0d2d82208794c2befd2f5fa0fe265f59` | `d27675d030a75a9e3c85912e260ffd7e5b17bea8` | `5b4e4337db3a7b57e37f5e1eb9a83570635a8c88` |
| TUI patch release | `0dbda6af5a349242cfde68194dfb10f489a12ef6` | `be80bfe570358a6d9f408f321ffe817a9acec733` | `25346a11048d01c8e445ad0e9c95aa62269325b0` |

The coordinator published through the connected GitHub Git-data API, preserving
exact reviewed trees and advancing `dev` with the expected old head. No force
push, history rewrite, release tag or main merge was performed. Original local
source branches are preserved. The TUI manifest, lock and resolved metadata all
bind its 13 embedded Host packages to canonical Host `667f0923b827fea9b5fdb60f75a21f5369e44193`.

## Final implementation and ownership

1. **Query paste.** The baseline `App::on_paste` excluded Find, Sessions, Models
   and Agents. These four active fields now append pasted text and share the
   typed-query refresh path: reset picker selection, recompute Find matches and
   reveal the first match, and clear armed deletion on a changed Session filter.
   CRLF, lone CR and LF each become one space; TAB becomes four spaces. Existing
   terminal sanitization removes ANSI/OSC/BOM and visibly escapes other controls
   and bidi controls. Ordinary Unicode is retained. Normalized-empty paste is a
   no-op. Paste never accepts a row, submits a prompt or edits the hidden draft.
   Existing composer, Palette, Files, unconfirmed-input, native interaction,
   fatal-state and read-only boundaries remain; rename-field editing is excluded.
2. **Forward-delete history.** The baseline unselected `Composer::delete`
   lacked a pre-edit snapshot. Every actual forward deletion now snapshots the
   complete edit state, creating a separate undo step and invalidating stale
   redo. Both physical Delete and nonempty Ctrl+D use that primitive. End-of-text
   no-op deletion preserves redo. Selected deletion remains one step; text,
   graphemes, complete paste chips, cursor and selection restore together.
   Empty Ctrl+D retains its explicit quit behavior.
3. **Session-picker identity.** The baseline showed short IDs only when full
   labels were exactly equal; subsequent clipping could erase the distinction
   between different long titles. Every row now starts its details with the
   existing six-character native Session-ID prefix. The Session row layout
   bounds oversized details, preserving that prefix before optional detail.
   Full-label duplicate counting is removed. Stored titles and full native
   `(agent_id, session_id)` action targets remain unchanged; no collision
   registry, backend fields or new ownership logic is introduced.

Host -> Agent -> SessionController and Runtime boundaries remain intact. All
468 tracked Host Rust files, including tests, are unchanged from the baseline;
Host changes are specification, documentation and version/lock synchronization.
The 225 Host and 272 TUI third-party locked package entries are unchanged.
README, architecture, key help, configuration/release references and changelog
were synchronized without altering the excluded behavior.

Workers were requested with `gpt-6-astra` / `max`; these requested settings are
recorded without independently attesting the effective runtime. The specification
checkpoint preceded production work. Contributors owned disjoint editor,
query-routing and picker-rendering paths; the coordinator handled shared release
files, canonical pins and publication. Independent final source review found no
remaining blocker. No Codex CLI or coding-model subprocess was used.

## Local validation

The complete Host gate passed: 1,255 uniquely enumerated Rust cases, seven
doctests, 28 Python checks, formatting, strict Clippy, strict rustdoc and
repository whitespace checks. The unique count uses `--list`; nested storage
subprocess output is not counted as additional tests.

The final canonical-pin TUI driver exited 0: 509 tests passed and four existing
cases were ignored, for 513 enumerated cases. The ignored cases are three opt-in
performance tests and the production diagnostic subprocess helper invoked by its
integration cases. This is 16 net additional regression cases from the 0.12.3
baseline. All 54 snapshot checks passed; those checks are included in the suite
and are not added again to its unique total. The focused editor module passed
16/16, including seven new routing/interaction/key cases. Primitive deletion
and actual-width rendering tests cover the remaining new behavior.

The first test compilation exposed two new fixture-only type errors (reusing a
non-Copy Panel and constructing Fatal as a String). Both were corrected in the
test file before the focused and final passing runs; production behavior was
unchanged. This record does not report that initial failed command as a pass.

Commands used by the coordinator and focused worker:

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked --offline -- --list
cargo fmt --all -- --check
cargo test --locked --offline --lib app::tests::editor
UPDATE_SNAPSHOTS=1 cargo test --locked --offline --lib ui::snapshots
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline --all-targets
cargo test --locked --offline --doc
RUSTDOCFLAGS='-D warnings' cargo doc --locked --offline --no-deps
cargo tree --locked --offline -p crossterm
cargo test --locked --offline --all-targets -- --list
```

The existing Rust 1.90 toolchain and locked dependency graph were used. The
final TUI dependency tree contains one crossterm version, 0.29.0. Validation logs,
test lists, publication mapping and metadata are under
`/workspace/scratch/ea58f4af9322/validation-0124/`. The independent final release
audit, `release-audit-final.json`, passed 53/53 metadata/provenance/frozen-evidence
checks. It inspected existing validation evidence rather than rerunning tests.

The local executor has only the Linux target installed, so the separate
`x86_64-pc-windows-gnu` cross-target Clippy command was not run. The published
Windows CI jobs build and test the native MSVC target; their result does not
claim GNU-target or manual Windows-terminal coverage.

## Native-terminal verification and limits

Fresh runtime: `/workspace/scratch/ea58f4af9322/tui-fixes-0124-20261115/`.
Its Chinese `RESULTS.md` links the retained phase records, screenshots, PTY
input/output, checkpoints, actual API log and shutdown evidence. The coordinator
operated the desktop; the launcher recorded PTY data without driving input.
The provider was an explicitly local simulated fixture, with real embedded
Host ownership, persistence and restart. No live-model quality or performance
validation is claimed.

Three native phases exited 0, with three Host starts/stops and two restarts.
The round retained 66 JPEG screenshots and 177 logical desktop actions. Phase 01
used the reviewed local-Host-pin build. Phases 02 and 03 used the canonical-pin
release binary with independently verified SHA-256:
`7537758cef31f5caf11fc2d9d7fcddba1af2f2006c2461cf739653d9ad568cda`.
The phases are identified separately rather than presenting the first binary as
the published-pin binary.

Observed UI subcases include all four pasted queries while idle and during
execution, Unicode Find and append, draft preservation, Delete and Ctrl+D
undo/redo, stale-redo invalidation and no-op redo preservation. Two distinct long
same-Agent titles retained visible different short IDs while idle/unloaded and
running, including with another Agent's Session foregrounded. Current and
noncurrent renames retained identities and affected the intended Session.

Exactly five explicit prompts account for five native admissions, five Turns
and five provider requests: one initial smoke completion, two initial timeout
outcomes, and two canonical successful completions. Query, rename and editing
exercises produced no extra admission or API request. The two initial
120-second simulated streams reached the configured 120,000 ms HTTP deadline;
they remain persisted failed/Timeout records and are not normal-completion
passes. The unchanged fixture configuration was re-exercised with 45-second
streams on the canonical binary.

Canonical A (`xe6jxt6rqkwc2mx9gzjc`) and B (`a35mt8r9mwbv15cw8qdq`) belong to
Coder Agent `jy7qh1v621s85t2z4d1w` and overlapped for about 24.559 seconds. Both
completed and persisted, in 45,164 ms and 45,176 ms respectively, each with one
provider request, one model attempt and zero tool rounds. Their complete saved
outputs contain lines 1–45 plus the late/end markers. A retained
`alpha-renamed-active` at configuration revision 2; only B's title/revision
changed in canonical phase 02, to `beta-renamed-canonical` at revision 3.

Checkpoint 02 -> 03 preserves metadata/history/API bytes across the first
restart. Checkpoint 04 -> 05 preserves all Session metadata/history and the API
log byte-for-byte across the final restart, with no new inputs, Turns or
requests. File checkpoints alone are not proof of live UI state or exact rename
timing; the desktop observations and native Turn intervals provide those
complementary boundaries. Fixture SIGTERM and a closed loopback endpoint are
recorded; a subsequent independent read-only check found no fixture PID or
listener. No cleanup success is inferred from a process wait timeout.

## Completed CI and evidence record

The coordinator fetched both workflow and job conclusions on 2026-11-15 at
13:51:04 UTC and retained them in `validation-0124/ci-results.json`:

- Host [run 43718763910](https://github.com/zqcli/minicore-host/actions/runs/43718763910)
  for `667f0923b827fea9b5fdb60f75a21f5369e44193`: completed/success, all nine jobs
  successful. Quality and native Linux, macOS Intel/Arm and Windows checks passed;
  Linux x86_64, macOS x86_64/arm64 and Windows x86_64 packages were built.
- TUI [run 43720128579](https://github.com/zqcli/minicore-tui/actions/runs/43720128579)
  for `be80bfe570358a6d9f408f321ffe817a9acec733`: completed/success, all eight jobs
  successful. Quality and native Linux/macOS/Windows Clippy/tests passed;
  the same four platform package targets were built.

The finalized Chinese report is `tui-fixes-0124-20261115/RESULTS.md`; its linked
screenshots, PTY records, native histories, checkpoints, API and cleanup records
remain in the fresh runtime directory. `validation-0124/report-review.json`
records an independent factual review of the result draft before the final CI
status update; no blocking discrepancy was found. Its document hashes identify
that reviewed draft, not this later CI-closeout text. The coordinator's final
delivery verification records the finalized document and publication hashes.

## Preserved exclusions and earlier evidence

The independent Web client, light-theme contrast, F4 mismatch, clipboard
readback, Chat Completions/Responses history limitations, missing diagnostic
phase, error discoverability, Session rename caret editing and the other
unapproved QA findings remain outside this repair. This focused native run does
not claim complete tool-workload, live-provider, long-run resource or all-platform
UI coverage.

Frozen earlier QA is retained unchanged, including its historical failure and
authorization wording. The final release audit independently verified these
original hashes:

- `qa/longrun/RESULTS.md`:
  `d48b6fd718e4336a9e5b560d9eddc44543398fa6c9cc5cbc0326494abd4e3d4c`.
- `qa/functional/RESULTS.md`:
  `132098a379a6782cb035510edc850e701af886f1447783d0eff473ef319fa6c2`.
- Earlier long-run archive:
  `0de472704cbdd335c645d22c3d1f816bb0186c6723ea65445c4d9534577276cd`.
- Earlier functional archive:
  `7fedb4b0f9b2057a61c6ada9fbd4e19e60041dbfc8b469fc15f7325b8b684569`.
- Earlier Session-name archive:
  `711d7478ca385d95e99ea2e62ad02a83da2f8b58268a56b81bf2c0bf756e7da1`.
