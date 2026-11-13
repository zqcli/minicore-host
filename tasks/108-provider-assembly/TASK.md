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
- The release baseline probe is compiled from the untouched dev source. It has
  not been executed. Full workspace validation, publication and the formal
  performance comparison remain pending; no performance result is claimed.

## Evidence and publication

Only source, documentation and reproducible measurement code belong in the
publication. Local raw measurement records are retained separately. Record
exact validation commands, source identities, CI outcome and performance
limitations here as work completes.
