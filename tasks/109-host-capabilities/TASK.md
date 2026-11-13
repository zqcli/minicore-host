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
- Implementation: pending.
- Validation: pending. No live provider or TUI validation is claimed.
- Publication and tag: pending.
