# Task 116: upstream errors and agent invocation context

## Goal and authorization

On 2026-11-15 the owner approved the source review's remaining repairs and
requested `gpt-6-astra` workers with `max` reasoning, no Codex CLI, a 0.0.1
increment for Host and TUI, and an updated specification. The missing agent
declaration was a user configuration issue; tool enablement changes and
`multi_tool_use.parallel` handling are excluded. Existing authorization permits
publication to `dev`; no main merge or release tag is requested.

## Starting checkpoints

- Host: `2f149145304c7cf6fd3271da937000b6e8593cbd`, version 0.12.2.
- TUI: `a7b2c0fab2fe43aa56a404a52364a4e351c2096b`, version 0.12.2.
- Target: Host and TUI 0.12.3, SPEC 4.11.2.
- Object and history formats remain schema 3.

## Accepted scope

- Preserve safely redacted upstream messages and actual status/type/code/request
  ID through HTTP/SSE, Runtime normalization, public Faults, compaction and the
  existing terminal history reason. Keep delivery and safe retry boundaries.
- Distinguish unknown upstream causes from recognized overload. Keep diagnostic
  logs metadata-only and never copy arbitrary response objects into Faults.
- Give selected builtin agent calls current Host/Agent/Session identities from
  the bound owner context. Preserve explicit targets and actual capabilities.
- Return useful agent parameter validation causes separately from policy denial.
- Preserve TUI startup/reload/shutdown and operation error details, existing
  failure/history/export display, and improve detail interaction and previews.
- Reduce full-message wrapping and full JSON pretty-print allocation for bounded
  previews without changing canonical output, history or export.

## Ownership

Three workers were explicitly requested through the collaboration API with
model `gpt-6-astra` and reasoning effort `max`: upstream error propagation,
agent context/validation, and TUI error presentation/previews. These are the
requested settings; no independent runtime-model attestation is claimed.
No Codex CLI or coding-model subprocess is part of this workflow.

Workers own disjoint source paths. The coordinator owns SPEC, version/lock
integration, independent review, validation records and publication. Runtime
does not depend on Host owner crates, and existing direct ownership remains.

## Validation plan

- Loopback HTTP/SSE error fixtures cover ordinary and partial failures,
  recognized quota/overload and unknown causes, actual upstream metadata,
  credential redaction, compaction, history/reopen and export.
- An agent fixture derives its first target only from the actual outgoing model
  request, then verifies real child acceptance and completion notification.
- Missing/invalid action fields and actual read-only/approval refusals remain
  distinct; excluded configuration and synthetic wrapper behavior are unchanged.
- TUI checks cover details, folding/copying, interaction targets, long text and
  bounded JSON previews while canonical content stays complete.
- Run locked formatting, strict Clippy, tests, doctests, strict rustdoc,
  repository scripts, test enumeration and existing platform/package CI.

## Progress

SPEC and scope are recorded before the implementation checkpoint. Implementation
and validation are in progress; no passing test or publication result is yet
claimed. Exact commits, commands, outcomes and limits will be recorded here.
