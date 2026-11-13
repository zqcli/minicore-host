# Task 108: simplify provider decoding and tool argument assembly

## Scope and authorization

On 2026-11-13 (Asia/Shanghai), the owner approved the reviewed two-stage design,
authorized its implementation and publication to dev, and requested a new
performance comparison after publication. The exact source baseline is dev
`913b736bd4e3d72ef5f9a9a7d9d0b381925a6a7a`. Host remains 0.10.1. SPEC 4.10.1
receives the approved implementation and partial-result clarifications; its
Rules and existing durable record structure are unchanged.

## Design and implementation boundaries

- Decode ordinary provider frames directly into typed messages. Preserve
  explicit error priority, first-event delivery facts and safe retry behavior;
  reject duplicate recognized fields without an alternate success decoder.
- Runtime alone accumulates raw tool arguments and parses each completed call.
  Provider retains only protocol metadata and fragments not yet handed off.
- Keep Start/Delta/End events, native value validation, cancellation, deadlines,
  response completion and EOF checks. Do not normalize JSON argument text.
- Hand off existing provider replay metadata before tool ends. Partial output
  can contain previously validated calls and later unfinished calls, including
  protocol-complete replay metadata, but cannot execute or become full history.
- Release successful raw argument buffers only after native ToolCall validation.
  Keep invalid arguments exactly as received. No new quotas, caches, compatibility
  layer, fallback success, provider-specific Runtime logic or dependency update.

## Plan and ownership

Root owns the SPEC clarification, integration, shared manifests, task/project
records, validation and publication. Provider and Runtime contributors own
disjoint production and regression-test paths. A third contributor prepares
reproducible local fixture measurements and independently reviews the change.
Workers are requested as `gpt-6-astra` with `max` reasoning; this records the
requested configuration, not independently attested effective model settings.

1. Preserve the untouched dev source and update the SPEC before production edits.
2. Implement and review both changes with targeted error/partial regressions.
3. Run the repository gate and publish verified code to dev without rewriting
   history. Track native multi-platform CI on the exact published source.
4. After publication, compare baseline and published production source with
   identical ordinary-text and tool-argument fixtures in balanced paired runs.
   Preserve all formal samples and report both gains and regressions.

## Progress

- Verified remote dev is the baseline above; the implementation worktree and
  detached baseline worktree are clean copies of that exact source.
- Added SPEC clauses for typed decoding, single raw-argument ownership,
  partial metadata and unchanged execution/retry boundaries.
- Implemented typed provider decoding and single Runtime argument assembly.
  Argument deltas transfer owned strings, the first fragment is adopted without
  copying, successful native call validation releases the raw buffer, and failures
  keep original JSON text. Protocol metadata precedes individual tool ends.
- Preserved provider error priority, first-event delivery and retry behavior.
  Direct decoding also validates whole-frame UTF-8 before typed deserialization;
  ignored extensions use the JSON library's skip semantics rather than a general
  validation tree. Duplicate typed-message/error-envelope fields are rejected;
  the error payload's existing classification behavior remains unchanged.
- Provider focused checks pass 8 unit cases, 1 diagnostics case, 42 real loopback
  HTTP cases, 1 doctest and strict Clippy. Runtime passes 143 cases, 2 doctests
  and strict Clippy. The additional Session regression proves that a partial
  with protocol-complete replay and an earlier valid tool never executes tools,
  survives real persistence/readback and stays out of full model history.
- Provider and Runtime contributors independently reviewed each other's changes;
  the measurement contributor also reviewed Runtime ownership and partial paths.
  No remaining blocking issue was identified within the approved scope.
- The full repository gate passes 1,126 unique Rust cases, 6 doctests and 28 Python
  checks, formatting, strict Clippy/rustdoc and whitespace. Test counts come from
  the separate all-targets listing, not nested subprocess output. All 225 external
  locked packages are unchanged. See [validation.json](validation.json).
- Published the four scoped commits by a leased fast-forward to dev, ending at
  `37295d788e76929a47c400899c91b152c83604f5`. Every remote tree matches its tested
  local tree; prior history is preserved and no remote topic branch was created.
- After publication, built the exact published candidate and executed six
  balanced AB/BA pairs with the identical release probe. All 48 processes,
  6,360 measured calls and 48 warmups are retained; complete outputs match.
  Independent verification recomputed source identities, checks and statistics.
- Local stream-processing median costs decrease 23.5%, 38.2%, 42.5% and 29.5%
  for text, small tools, large arguments and many tools respectively. Large and
  many-tool process peak RSS decrease 36.9% and 21.7%; text/small-tool RSS increase
  42/44 KiB. Individual spikes remain, so no tail-latency improvement is claimed.
- See the [result report](REPORT.md). Production Rust grows by 46 physical lines
  including comments/blank lines; repeated runtime work decreases, not total
  source size. Raw samples/build/environment records are delivered separately.

## Evidence and publication

Only source, documentation and reproducible measurement code belong in the
publication. [Publication mapping](publication.json) records tested/published
trees and exact sources. [Native CI run 43348681669](https://github.com/zqcli/minicore-host/actions/runs/43348681669)
tracks Linux quality, Linux/macOS ARM64/macOS x64/Windows tests and all four
native packages for the measured implementation. Subsequent result documentation
does not change production code, dependencies, tests or workflow configuration.
