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

## Implementation

- Provider extracts the known message and source fields from HTTP failures,
  Chat SSE, Responses errors/failed responses and incomplete reasons. It redacts
  the credential actually bound to the request before creating a ModelError.
  Unicode, LF and TAB stay readable; other control characters are escaped.
- Runtime keeps optional boxed ModelUpstreamDetails. Observed-output
  normalization and serde retain the source. Host maps the same safe text to
  Fault.message and the existing durable terminal reason. Fault 4210 and 3702
  can carry the existing upstream-details schema with the added status/ID fields.
- Unrecognized provider causes use UpstreamUnclassified/4299. Existing explicit
  delivery evidence, retry ownership, cancellation and operation deadlines remain.
  A real HTTP 408 in compaction retains the source rather than becoming a generic
  timeout string. Unstructured bodies are not dumped into business errors.
- Selected builtin agent prompts bind the current owner-minted identities once,
  after Workspace and before history. This does not grant capability or invent a
  target. External tools named agent and compaction do not gain this context.
- InvalidInvocation and explicit Failed tool outcomes distinguish malformed
  arguments from actual policy/approval denial. Safe field/constraint reasons
  reach failed tool results; arbitrary invalid values and unknown names do not.
- TUI retains startup/reload/shutdown causes, offers /error for the latest full
  operation error, and opens failure details from the transcript. Its collapsed
  failure wrapping and non-bash JSON head previews avoid allocating discarded
  output; complete results remain available through native history and export.

## Independent review

The Host workers reviewed one another's changes. Review identified and repaired
two specific issues before publication: HTTP 408 source loss in compaction, and
a forged `, expected ...` suffix in an unknown field of the empty close_scope
variant. Both now have focused regressions. Runtime's new source is boxed to
preserve the small common error representation and strict Clippy checks.

The agent worker also independently reviews the TUI. Its first pass identified
async rename/send error entries which already knew their original target but
did not bind that target to the new error view. Those entries and the same
existing delete/default/export boundaries now retain their actual identities,
with cross-Session message-dispatch regressions. Agent-operation failures retain
their captured Agent ID. No control or acceptance lifecycle is added.

## Validation and publication

SPEC-first checkpoint: `d6d6a40af081bc91d649c03dd57ccb9a44bed2d0`.
Host and TUI manifests/lockfile packages are 0.12.3. A structured comparison
confirms all 225 Host and 272 TUI external locked packages are unchanged.
SPEC 4.11.2 SHA-256 is
`19fab2cf90270200430ca1b229a4839bfc17ea2862ea60a22f8e87ae3cdfbd56`.

The full Host local gate passed: 1,255 uniquely enumerated Rust tests, seven
doctests and 28 Python checks, formatting, strict Clippy and strict rustdoc.
Commands: `./scripts/check.sh` and
`cargo test --workspace --all-targets --locked --offline -- --list`.
The TUI local Host-path integration gate passed before the last target-context
regressions: 491 default tests passed and four existing opt-in/helper cases
were ignored (495 enumerated), with formatting and strict Clippy/rustdoc.
The final TUI gate will use the immutable published Host revision without a
local path patch and include the last target-context regressions.

Validation uses
Rust 1.90.0, ordinary uid 1000, locked dependencies and a validation-only Linux
subreaper so container PID 1 cannot strand process-cleanup test children. No
production cleanup or test assertion is relaxed. Provider integration uses
explicit loopback fixtures, not live credentials or live provider service.
Exact implementation commits, final TUI test counts and platform/package CI
outcomes follow after publication. No complete remote CI result is yet claimed.
