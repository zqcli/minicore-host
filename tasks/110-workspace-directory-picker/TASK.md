# Task 110 — Host workspace directory picker

## Authorization and baseline

- Owner authorized separate minicore-web with its existing UI and native Host WSS;
  add `workspace.roots`, `workspace.list`, `workspace.mkdir`, then release Host 0.11.1.
- Host input: dev `32f074c57868533af81b0c6491440bae24b872c8` (0.11.0).
- Design first: SPEC 4.10.3, SHA-256
  `a9bec7b69a94082e7ea4d156f82f58361c241cb0d7ed9c01935f80b3bee9b792`.
- Mature directory reference: minicore-tui
  `2451d7289214385e3d21b46174b36f32a41dce45:src/remote/fs.rs`; its fixed
  MAX_ENTRIES quota is excluded under SPEC Rule 12.
- Requested engineering worker: gpt-6-astra, max reasoning. Effective runtime
  configuration is not independently attested.
- Follow-up authorization: fix the existing compaction declaration ordering
  defect exposed by the coordinator's real Web/Host integration.

## Published commits and ownership

The public `dev` history preserves SPEC-before-implementation order:

1. `19390dfef43944ec03840e7cabafd960a1d447a7`
   `docs(spec): define Host workspace directory selection` — committed before code;
   local preparation `2dc0cd31a8116b65b5ace6d4c71d0a1c193413ee`.
2. `d03e5c1d2c39b9ff5da6bc19c692f96540e8b10b`
   `feat(workspace): add Host directory picker operations` — source, tests,
   component documentation, configuration templates and workspace 0.11.1;
   local preparation `c9cc848959fe8882d5925e46a00c43fb677f0efd`.
3. `a0b560ee59c9b04057361ede2757e99f21295613`
   `fix(resources): sort compaction tool declarations` — existing contract fix
   and strengthened real-owner manual/automatic compaction regressions;
   local preparation `06ab698ef4cc37d74984b8937b5284dbd5a94113`.
4. `f6076cda5584799747b284ff0e1c63985a16325a`
   `docs(project): record workspace picker validation`.
5. `1b347046c5bdc752c40fc7636cdd14e34a04b8cb`
   `test(workspace): isolate non-UTF-8 directory fixture`;
   local preparation `82f8077c558609ac47d45cbe2f3978443efe363a`.
6. `b20f6fee1461cec2925d2538e6d4747c79e9c2b4`
   `test(core): settle activity saves before history fault injection`;
   local preparation `6bab4a7898f2194977f47a2aac4625cac1997d26`.
7. `d58e1af6dbb01b0056afccd9f51d183cdefecc98`
   `test(core): isolate history fault fixtures from activity saves`;
   local preparation `d9afcee8b07e4761e76759bb6755af9dfa66c5a1`.

The final verified public source is `d58e1af6dbb01b0056afccd9f51d183cdefecc98`, tree
`44f2db3364fc1f49f0e7f98da91e5885d032d5a8`, matching its local preparation exactly.
The coordinator owns publication and separate Web/Host integration. Native CI
and package evidence below belongs to this exact public source. This completion
record does not attest a remote `v0.11.1` tag.

## Implemented behavior

Host configuration owns explicit `workspace_roots`; default empty exposes none.
Preparation validates and canonicalizes the directories, and existing Host reload
publishes them atomically with its other configuration. The directory scope only
applies to the three picker methods; Session workspace selection and tool policy
retain their existing boundaries.

Host owns authentication, current operator admission, accepted mkdir work and its
receipt. `host-workspace` performs actual filesystem operations; the shared typed
operation table supplies direct, stdio and WSS access without a Session. Roots/list
require a valid reader. Mkdir creates one child, preserves existing-entry and other
real errors, and requires the current operator. The Host retains the actual worker
through caller abandonment/disconnect and shutdown, storing creating/created/failed
facts in the existing receipt collection. Operation IDs remain correlation only.

Lists contain actual child directories and allowed symlink targets, with caller
page size and scoped cursors. They do not inherit the old fixed 1,000-entry limit.
Returned paths retain Host-platform semantics and reject lossy UTF-8 conversion.
Windows ordinary drive/UNC paths are simplified only when safe; volume GUID and
verbatim-dependent names retain their native prefix. Existing-name cursor checks
are separate from mkdir's Windows creation-name restrictions, so a returned cursor
can continue after an existing trailing-dot/space directory. The native Windows
fixture verifies that continuation and actual roots/list/mkdir path round trips.

Compaction now sorts its cloned tool declarations by name, matching Runtime's
existing strict request invariant. Session configuration order remains unchanged.
The two existing real Host + loopback HTTP/SSE compaction cases now select two tools
in reverse order and verify successful manual/automatic compaction, normal-request
ordering, and no utility tool execution or extra Turn-bound tool preparation.

There is no compatibility gateway, implicit root, application quota, retry,
generic operation manager, changed connection takeover rule or changed Turn
approval policy. Runtime ownership and TUI behavior are unchanged. Local path
containment retains the documented same-user concurrent path-replacement boundary;
this work does not introduce an OS filesystem sandbox.

## Initial local validation — Linux, Rust 1.90.0

Commands used `/home/agent/.cargo/bin/cargo`; build/test/lint commands used
`CARGO_BUILD_JOBS=4`. Component case counts below are run results, not a claimed
full-workspace unique case count.

| Command | Result |
| --- | --- |
| `cargo test -p minicore-host-protocol --all-targets --locked` | 79 passed, including all 55 catalog routes and strict directory receipt decoding |
| `cargo test -p minicore-host-workspace --all-targets --locked` | 38 passed: 7 directory, 5 edit journal, 8 existing filesystem, 18 Git/change cases |
| `cargo test -p minicore-host-core --lib --locked` | 135 passed, including 4 new directory authority/ownership/reload cases |
| `cargo test -p minicore-host-transport --test directory_picker --locked` | 2 passed, including real TLS WSS and OS-pipe stdio/direct parity |
| `cargo test -p minicore-host-resources --test task023 --locked compaction::` | 2 strengthened real-owner compaction cases passed after the ordering fix |
| `cargo test -p minicore-host-workspace --lib --locked` | Final affected rerun: 7 passed after Windows path/cursor corrections |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed; rerun after the path/cursor and compaction fixes |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` | Passed for all 13 packages |
| `cargo test --workspace --doc --locked` | 7 passed |
| `cargo build -p minicore-host --locked` | Passed; final binary rebuilt after the compaction fix |
| `cargo fmt --all -- --check` and `./scripts/check-whitespace.sh` | Passed |
| Three Python checks from `scripts/check.sh` | 28 passed: projection 15, RSS 5, performance helper 8 |

The directory transport fixture checks initialization-before-business, invalid-key
refusal, actual operator mkdir, read-only and unauthenticated mkdir refusal, actual
file effects, receipt readback, pagination and outside-root errors. The core fixture
checks expired callers, no automatic promotion, duplicate operation IDs, and a
controlled real write surviving waiter abort, disconnection and force shutdown.
The filesystem cases include 1,003 directories, symlink containment, real common
errors, Unix names/non-UTF-8 refusal and Windows drive/UNC/volume/verbatim formatting.

The initial version check found the exact host-storage protocol pin still at
0.11.0; it was updated. The new malformed-receipt test found that a Serde unit
variant accepted unrelated fields; creating is now a strict empty struct variant
with identical valid JSON. Full-target compilation exposed three complete older
HostConfig test fixtures; each now explicitly sets empty workspace roots.
Independent review found the Windows prefix issue; the safe conversion and existing
cursor-name correction are both included in the directory implementation commit.

All 13 workspace packages and internal pins are 0.11.1. A parsed Cargo.lock
comparison against the input commit confirms all 225 external package entries,
versions, sources and checksums are unchanged.

## Native CI fixture follow-up — 2026-11-14

Follow-up input: dev `f6076cda5584799747b284ff0e1c63985a16325a`.
The coordinator inspected [native CI run 43553519487](https://github.com/zqcli/minicore-host/actions/runs/43553519487):
Linux quality checks passed with 1,192 unique Rust cases, 7 doctests and 28 Python
checks; Windows native and ordinary-user checks also passed. Both macOS jobs failed
in the same Unix test fixture while `fs::create_dir` tried to create the byte name
`[0xff]`: OS error 92, `Illegal byte sequence`. The failure preceded Host listing;
it did not demonstrate a production directory-operation failure.

The fixture correction splits the combined case into three tests. All Unix targets
retain native valid-name creation and an explicit `display` rejection of an
in-memory non-UTF-8 path. Linux separately creates the actual non-UTF-8 directory
and requires `list` to return `DirectoryError::NonUtf8Path`; fixture setup and the
failure assertion remain strict. Production code, SPEC, manifests and dependencies
are unchanged. The requested worker configuration remains gpt-6-astra with max
reasoning, without independent attestation of effective runtime settings.

Local follow-up validation uses Rust 1.90.0 and `/home/agent/.cargo/bin/cargo`:

| Command | Result |
| --- | --- |
| `CARGO_BUILD_JOBS=4 cargo test -p minicore-host-workspace --lib --locked` | 9 passed, 0 failed, 0 ignored; includes all three separated cases |
| `CARGO_BUILD_JOBS=4 cargo clippy -p minicore-host-workspace --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all -- --check`, `./scripts/check-whitespace.sh` and `git diff --check` | Passed |

No full-workspace relink or macOS execution was claimed locally. This correction
was published as `1b347046c5bdc752c40fc7636cdd14e34a04b8cb`; the later native runs
below establish its actual platform results. The first run's Linux count belongs
only to that initial revision.

## Second native CI — history fixture ordering

The published directory-fixture correction is
`1b347046c5bdc752c40fc7636cdd14e34a04b8cb`, tree
`ba7df7eb810f82f4b7e34fcfac1f79e054da0fbd` (identical to local preparation
`82f8077c558609ac47d45cbe2f3978443efe363a`). Its
[push CI run 43555557066](https://github.com/zqcli/minicore-host/actions/runs/43555557066)
passed Linux quality and native Linux, macOS ARM64 and Windows checks. The three
successful native jobs' separate `cargo test --workspace --all-targets --locked -- --list`
steps contain 1,194, 1,186 and 1,171 unique cases respectively; each passed 7
doctests. Counts match both the binary-plus-case identities and the list summaries;
nested execution output is excluded. Linux quality passed all 28 Python checks,
formatting, strict Clippy/rustdoc and workspace tests. Windows additionally passed
all 5 standard-user cases with `IsAdministrator=false`, `ProfileLoaded=true`,
`ExecutableWriteAccess=denied` and `StandardUserExitCode=0`.

Both macOS jobs passed all 8 directory tests, including native names and non-UTF-8
display rejection. Intel macOS instead failed the existing
`resume_settles_history_and_metadata_originals_without_cross_confirmation` fixture
at `history_continuation.rs:446`: its single configuration update returned
`SessionBusy` instead of the intended `SessionHistoryWriteFailed`. Its subsequent
doctest/list steps and all package jobs were skipped; no successful package or
complete native gate is claimed for this run.

Source inspection and independent review identified a fixture ordering race.
Model admission did not mean the accepted-input activity checkpoint had finished.
`SessionStore::load` also opens `history.jsonl`, so the test's rename/replacement
gap could fail that still-running metadata I/O. The failed owner work remains
retained with `running=false`; waiting only for `!metadata_saving` therefore did
not establish that a new configuration update could be admitted. The correction
waits for `!owned_work.metadata_save` while the model gate is still blocked,
before replacing the history path, and uses the same owner condition before
injecting the configuration-save failure. Both waits explicitly reject an
unexpected metadata fault. The original single update, exact failure assertion,
explicit resumes and no-rerun checks remain unchanged; there is no sleep, business
retry, ignored error or production change.

Local validation from public input `1b347046c5bdc752c40fc7636cdd14e34a04b8cb`:

| Command / check | Result |
| --- | --- |
| `CARGO_BUILD_JOBS=4 cargo test -p minicore-host-core --test history_continuation --locked` | All 4 cases passed |
| Compiled `history_continuation` binary, `--exact resume_settles_history_and_metadata_originals_without_cross_confirmation --color never`, 200 independent executions with 4 concurrent processes | 200 passed, 0 failed |
| `CARGO_BUILD_JOBS=4 cargo clippy -p minicore-host-core --test history_continuation --locked -- -D warnings` | Passed |
| `cargo fmt --all -- --check`, `./scripts/check-whitespace.sh` and `git diff --check` | Passed |

The unmodified case also passed 100 local Linux executions with 4 concurrent
processes; this host did not reproduce the Intel macOS failure. The identified
source race and native failure are distinct evidence from the focused local
successes. This second test-only correction was published as
`b20f6fee1461cec2925d2538e6d4747c79e9c2b4`, with the exact tree
`e7af7f818958ff0ecd3a89aef4fe15e75023063a` of local preparation
`6bab4a7898f2194977f47a2aac4625cac1997d26`. Full-workspace linking was not repeated
locally; its native run is recorded below.

## Third native CI — isolate history fixture replacement windows

[Push CI run 43557234311](https://github.com/zqcli/minicore-host/actions/runs/43557234311)
ran against public source `b20f6fee1461cec2925d2538e6d4747c79e9c2b4` in attempt 1.
All four native jobs passed. Their independent `--list` steps counted 1,194 cases
on Linux x64, 1,186 on each macOS architecture and 1,171 on Windows x64, matching
both unique binary-plus-case identities and list summaries. Every platform passed
7 doctests; Windows also passed the 5 standard-user cases with a loaded profile,
non-administrator identity, denied executable writes and exit code 0. The prior
Intel macOS history-continuation failure and both Unix directory tests passed in
the actual Intel log. These are this run's results, not reused earlier outcomes.

The separate Linux quality job `132332725703` passed 28 Python checks, formatting
and strict Clippy, then failed `host_shutdown` with 13 passed and 1 failed:
`shutdown_force_failed_history_holds_root_until_original_save_continues` exceeded
the existing three-second `until` deadline. CI reports shared helper line 162
without a backtrace; it does not establish whether the wait for failed history or
the later wait for Host stopped timed out. The same case passed in every native
job. The package matrix was skipped, so the complete gate remains unsuccessful.

Source inspection and independent review confirmed two uncontrolled filesystem
windows in that fixture: initial history replacement and restoration before its
single resume. Both can overlap activity metadata I/O that also opens the history
path. A failure reported after resume selects original work can retain metadata
work and prevent shutdown; that is a source-supported race, not an observed CI
call-site trace. The bounded review found identical windows in the first history
continuation case, two tool-cleanup cases and one persistent-owner case. The
follow-up adds nine owner barriers across those five cases in four test files,
before their specific history replacement/restoration operations. Every barrier
requires `!owned_work.metadata_save` and explicitly rejects an unexpected metadata
fault. Model/tool gates keep the intended execution blocked during initial fault
injection. The test that intentionally retains a configuration metadata failure
keeps its existing semantics and is not required to clear that work.

Production, timeouts, original fault/data assertions and single business calls
are unchanged. The new waits use existing yielding/state observation mechanisms;
they add no fixed sleep or retry. Local validation on Rust 1.90.0:

| Command / check | Result |
| --- | --- |
| `CARGO_BUILD_JOBS=4 cargo test -p minicore-host-core --test host_shutdown --test history_continuation --locked` | 14 shutdown and 4 history cases passed |
| `CARGO_BUILD_JOBS=4 cargo test -p minicore-host-core --test task033_tool_cleanup --test persistent_owners --locked` | 12 tool-cleanup and 46 persistent-owner cases passed |
| Strict Clippy with the same two pairs of `--test` targets and `-- -D warnings` | Both commands passed |
| `cargo fmt --all -- --check`, `./scripts/check-whitespace.sh` and `git diff --check` | Passed |

All 76 affected-suite cases passed. This follow-up used no additional probabilistic
repeat batch or complete workspace relink. It was published as
`d58e1af6dbb01b0056afccd9f51d183cdefecc98`, and its complete native results follow.

## Verified native CI and packages — 2026-11-14

[Push CI run 43558286748](https://github.com/zqcli/minicore-host/actions/runs/43558286748)
completed successfully on attempt 1 against source
`d58e1af6dbb01b0056afccd9f51d183cdefecc98` and tree
`44f2db3364fc1f49f0e7f98da91e5885d032d5a8`. All nine jobs succeeded: the Linux
quality gate, four native test jobs and four native release package jobs.

The [Linux quality job](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132336006809)
ran the complete `./scripts/check.sh`: whitespace, Python checks (15 projection,
5 RSS, 8 performance helper), formatting, strict workspace Clippy, all-target
Rust tests, 7 doctests and strict rustdoc for all 13 workspace packages passed.
The previous shutdown timeout and every affected history/activity fixture passed
in this quality job and in all four native jobs.

Each native job ran strict workspace/all-target Clippy, full all-target tests,
doctests and its own `cargo test --workspace --all-targets --locked -- --list` step.
Counts below come only from those independent list steps. Binary-plus-case
identities, listed entries and per-suite list totals agree on each platform;
nested subprocess execution output and repeated standard-user cases are excluded.

| Native platform | Job | Unique Rust cases | Doctests |
| --- | --- | ---: | ---: |
| Linux x64 | [132336006824](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132336006824) | 1,194 | 7 |
| macOS ARM64 | [132336006749](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132336006749) | 1,186 | 7 |
| macOS x64 | [132336006524](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132336006524) | 1,186 | 7 |
| Windows x64 | [132336006776](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132336006776) | 1,171 | 7 |

Both macOS jobs actually passed native-name creation and non-UTF-8 display
rejection; Linux also passed actual non-UTF-8 directory/list refusal. Windows
passed the real verbatim-name roots/list/mkdir round trip. Its standard-user step
passed all 5 existing Storage/Host cases, with `IsAdministrator=false`,
`ProfileLoaded=true`, `ExecutableWriteAccess=denied`, `StandardUserTestsPassed=5`,
`StandardUserExitCode=0` and `TemporaryAccountRemoved=true`.

Every package job built Host with the internal Runtime for its native target,
ran the packaged executable's actual `--help` smoke test, checked the expected
stderr without protocol stdout, and verified the archive by reading it back.
Each archive contains exactly 20 files and one Host executable; the archived
binary SHA-256 matches the smoke-tested binary. Job logs retain both Host and
archive SHA-256 values. All four artifacts were uploaded and their registry
entries identify the verified source SHA above.

| Package target | Job | Uploaded artifact | Archive verification |
| --- | --- | --- | --- |
| `x86_64-unknown-linux-gnu` | [132338654171](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132338654171) | [Host 0.11.1 Linux x64](https://github.com/zqcli/minicore-host/actions/runs/43558286748/artifacts/14099342041) | 20 files, one Host executable; smoke and readback passed |
| `aarch64-apple-darwin` | [132338654257](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132338654257) | [Host 0.11.1 macOS ARM64](https://github.com/zqcli/minicore-host/actions/runs/43558286748/artifacts/14099142519) | 20 files, one Host executable; smoke and readback passed |
| `x86_64-apple-darwin` | [132338654172](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132338654172) | [Host 0.11.1 macOS x64](https://github.com/zqcli/minicore-host/actions/runs/43558286748/artifacts/14099117832) | 20 files, one Host executable; smoke and readback passed |
| `x86_64-pc-windows-msvc` | [132338654160](https://github.com/zqcli/minicore-host/actions/runs/43558286748/job/132338654160) | [Host 0.11.1 Windows x64](https://github.com/zqcli/minicore-host/actions/runs/43558286748/artifacts/14099377121) | 20 files, one Host executable; smoke and readback passed |

The coordinator reports independent real Web/Host WSS integration separately.
These Host checks use actual owners, filesystems and transports with controlled
model fixtures; they do not claim live-provider or browser-rendering validation.
The three preceding CI outcomes and their fixture corrections remain recorded
above. Native package availability does not establish a remote version tag.
The completion commit changes only this task record and `PROJECT.md`, uses
`[skip ci]`, and leaves the validated code/tests unchanged; `git diff --check`
passes without rerunning the completed test matrix.
