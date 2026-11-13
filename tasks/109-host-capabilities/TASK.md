# Task 109 — Host capabilities for the TUI client

## Authorization and baseline

- Owner request: update SPEC first by 0.0.1, implement Host capabilities only,
  exclude tool-argument generation observation, release and tag Host 0.11.0.
- Host input: dev `7a1881c85e57a97c85724c1a90a87852a1930bd8` (0.10.1).
- TUI reference: `3beeb1d8a525e58ce6352e05061f242c5580e248` (0.9.9), unchanged.
- Agent code reference: `f36a87366d8e2fe0a1c8216463887902e471c739`.
- Design first: SPEC 4.10.2, SHA-256 `55476e6d2c6c9ec5b857f2b9235834a162b3ad85ce56a8e2a295135f7dc370c7`.
- Requested engineering worker configuration: gpt-6-astra, max reasoning;
  effective runtime settings are not independently attested.

## Scope

1. Explicit Chat Completions / Responses model protocol and production Responses
   streaming, partial/error handling, exact tool identity and native replay.
2. Existing single-file apply_patch capability using actual Host workspace,
   policy, edit recording and cleanup owners.
3. Positioned history records, fixed forward/backward windows, actual terminal
   and compaction facts, and event/result/history correlation.
4. Session activity, input acceptance and history timestamps with real basic
   terminal counters, owned and saved at existing boundaries.
5. Independent review, full local gates, native CI, workspace 0.11.0 and tag.

## Exclusions

No TUI/Web changes, argument-generation observation, old-record/config migration,
new public RPC methods, quotas, implicit retries or execution recovery.

## Plan and ownership

- Root: SPEC, configuration/resource integration, release metadata, integration
  review, validation and publication.
- Responses worker: host-provider implementation, tests and component docs.
- Patch worker: host-local-tools implementation, tests and component docs.
- History worker: core/storage/history contract, activity facts and tests.
- Parallel code implementation starts only after the design commit.

## Progress

- 2026-11-13: baseline inspected; SPEC updated before implementation.
- SPEC-first commit: `32ea194ff2269aa8fb2bebff79355b600d404b45`.
- Responses implementation and tests integrated as `259926c` and `01b9ddf`;
  [provider evidence](responses.md) records 71 passing provider cases, including
  20 new Responses cases, and two compiling doctests.
- Patch implementation integrated as `c85b2e2`; [patch evidence](patch.md)
  records 48 passing local-tool cases, including 16 new patch cases.
- Root integration adds required `OpenAiModelApi`, safe model discovery,
  exhaustive provider construction, mutating patch policy, configuration
  templates and package inclusion. A separate worker reviewed these changes;
  its test-fixture correction was applied, with no unresolved source findings.
- Root focused checks: two configuration tests, 15 resource/local-tool tests
  and two real Host Responses/compaction/reload tests pass on Linux. The actual
  Host binary's read-only policy matrix also passes with a valid patch call.
- All 13 workspace packages and internal pins are prepared at 0.11.0. The 225
  external locked package versions, sources and checksums remain unchanged;
  provider reuses the existing workspace-pinned sha2 dependency.
- History implementation integrated as `83e3d8acca1a92cf8acd19167284f6bf37a6da7d`,
  from worker `bde301b6a138cf979b8b062006e8c64c87bf99bc`.
  [History evidence](history.md) records component results and their limits.
  Automatic compaction saves the already-bound input prefix before its marker,
  preserving public positions/times through append failure and reconciliation.
  Activity checkpoints register under their acceptance lock before publication;
  successful older saves register coalesced follow-on work before quiescence.
  Two independent reviewers closed the source findings. Root fixture commit
  `cf4bd05` waits for those actual owners before configuration/unload actions
  and asserts the intended write failure instead of accepting any error.
- Final worker protocol filter passed both timestamp/direction cases; the
  final corrected read-only history fixture also passed its narrow rerun.
- Object schema 2/history schema 3 and required model `api` are intentional
  configuration/data changes. Earlier envelopes are explicitly rejected; no
  timestamps, counters or execution identity are backfilled.
- The owner's follow-up requested immediate remote publication. Source was
  pushed to dev as `16a9561ef1fde410f853e1b986e93a342e73e176`, followed by fixture
  corrections at `aae95883bf03114ec19525c72bf8169ecc095295`. Both publications
  preserved remote history with leased fast-forwards. Git tree equality was
  checked after API publication and Git fetch; the
  [publication record](publication.json) maps local and remote commit identities.
- Full local `CARGO_BUILD_JOBS=4 ./scripts/check.sh`: passed on local
  `aae31329b3014e5cb9ea11e6787eca6fd3ad7ec6`, whose tree
  `94021940b08cc49e383a49135dcfe7ed48c92669` is identical to published `aae9588`.
  This includes formatting, strict Clippy/rustdoc, all workspace/all-target
  tests, seven doctests and 28 Python checks. The independent
  `cargo test --workspace --all-targets --locked -- --list` counts 1,178 unique
  Rust cases across 141 test targets; nested storage subprocess output is not
  counted again. [Validation evidence](validation.json) retains the commands,
  exact source, earlier failures and their corrections.
- The local Linux release build, actual staged-binary smoke test and archive
  readback pass: 20 files, one executable, both model configuration templates.
  Its exact documentation checkpoint, archive size and hashes are recorded
  separately from the final CI packages.
- The first native CI exposed fixture assumptions about native terminal and
  compaction records, accepted activity saves, and independent-run metadata.
  Windows also exposed a reserved-device filename and an aggregate long-Turn
  timeout. The test-only follow-up keeps exact same-execution identity and
  functional assertions. It waits for actual owner settlement and bounds each
  request-admission phase; it does not add production retries or quotas.
- Final [native CI](https://github.com/zqcli/minicore-host/actions/runs/43400316403)
  passed all nine jobs on published `aae9588`: quality, four native validation
  jobs and four release packages. Exact native case counts are Linux 1,178,
  macOS ARM64/x64 1,171 each, and Windows 1,157; each platform has 141 test
  targets and seven passing doctests. Windows also passed five existing cases
  under a standard user; these are not counted again. Every package passed the
  native staged-binary smoke check and archive readback with 20 files and one
  Host executable. [CI evidence](ci-source.json) records job and artifact IDs,
  distribution archive and executable hashes, and GitHub outer-artifact hashes.
  Those CI archives were checked by the native package helper; this record does
  not claim another local download/readback. No live provider or TUI validation
  is claimed.
- Remote tag: pending; source publication is complete.
- Publication capability check: authenticated terminal Git is unavailable
  (`git push --dry-run` cannot read a username; `gh auth status` reports no
  account). The available GitHub connector supports branch updates but no tag
  creation. The browser repository page is also signed out. No remote tag is
  claimed. The final evidence-only dev checkpoint is the intended tag target;
  the remaining action is an authenticated push of the authorized `v0.11.0` tag.
