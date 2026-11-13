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
- Full workspace validation and native CI: pending. No live provider or TUI
  validation is claimed.
- Publication and tag: pending.
- Publication capability check: authenticated terminal Git is unavailable
  (`git push --dry-run` cannot read a username; `gh auth status` reports no
  account). The available GitHub connector supports branch updates but no tag
  creation. The browser repository page is also signed out. No remote tag is
  claimed; finish implementation, checks and branch publication before asking
  for the final authentication step.
