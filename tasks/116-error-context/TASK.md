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

The agent worker also independently reviewed the TUI. Its first pass identified
async rename/send error entries which already knew their original target but
did not bind that target to the new error view. Those entries and the same
existing delete/default/export boundaries now retain their actual identities,
with cross-Session message-dispatch regressions. Agent-operation failures retain
their captured Agent ID. Final targeted review found no unresolved blocking
issue. No control or acceptance lifecycle is added.

## Validation and publication

Published SPEC-first checkpoint:
`2ce92dada73124478d8ba25a85b0958581cee019`.
Published Host implementation/TUI pin:
`dbaaa8175780759b84d26e0519aa4b27ced5f8f4`.
Published TUI implementation:
`0b7d4dc799fbe8c124661713bf8300fc3686ca78`.
The implementation's source CI is
[run 43688899390](https://github.com/zqcli/minicore-host/actions/runs/43688899390).
TUI's source CI is
[run 43689337362](https://github.com/zqcli/minicore-tui/actions/runs/43689337362).

The local CLI had no GitHub write credential. Publication used the connected
GitHub Git-data API, with author/committer `zqcli`, ordered specification and
implementation commits, and a non-forced `dev` ref update requiring the original
head. The specification tree `df3f259018867b249a53701a09fd514a6d40747e`
and implementation tree `0c2720b1306569e75c07231208994754091f3834` exactly match
the locally validated checkpoints `d6d6a40af081bc91d649c03dd57ccb9a44bed2d0`
and `406df68e22257f6b8d4ca730d7930fdaeab35fa4`. Fetch and a zero-diff comparison
verified the publication; those original local branches remain available.
TUI used the same publication procedure: its tree
`d5baf967b833e38b9807b4abfb42d86f266a6791` exactly matches the fully validated
local checkpoint `6e7c05cdf4b811ab081cbef1391dc80e41067234`; fetch and a zero-diff
comparison verified that result too. Both ref updates preserved the remote parent
history and created no release tag.

Host and TUI manifests/lockfile packages are 0.12.3. A structured comparison
confirms all 225 Host and 272 TUI external locked packages are unchanged.
SPEC 4.11.2 SHA-256 is
`19fab2cf90270200430ca1b229a4839bfc17ea2862ea60a22f8e87ae3cdfbd56`.

The full Host local gate passed: 1,255 uniquely enumerated Rust tests, seven
doctests and 28 Python checks, formatting, strict Clippy and strict rustdoc.
Commands: `./scripts/check.sh` and
`cargo test --workspace --all-targets --locked --offline -- --list`.
The final TUI gate passed against the immutable published Host revision, without
a local path patch, including all final target-context regressions: 493 default
tests passed, four existing opt-in/helper cases were ignored (497 enumerated).
The ignored cases are three optional performance measurements and the isolated
production diagnostic subprocess helper, which integration tests invoke
explicitly. Final commands were:

```sh
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline --all-targets
cargo test --locked --offline --doc
RUSTDOCFLAGS='-D warnings' cargo doc --locked --offline --no-deps
cargo tree --locked --offline -p crossterm
cargo test --locked --offline --all-targets -- --list
```

Strict lint/documentation gates passed, and the dependency tree contains exactly
one crossterm version. Both metadata and Cargo.lock confirm all 13 embedded Host
crates at 0.12.3 use the same `dbaaa8175780759b84d26e0519aa4b27ced5f8f4` Git source.

Validation uses
Rust 1.90.0, ordinary uid 1000, locked dependencies and a validation-only Linux
subreaper so container PID 1 cannot strand process-cleanup test children. No
production cleanup or test assertion is relaxed. Provider integration uses
explicit loopback fixtures, not live credentials or live provider service.
The exact source CI records are linked above. The handoff gate requires all nine
Host jobs (quality, four native platform jobs and four package jobs) and all eight
TUI jobs (quality, three native platform jobs and four package jobs) to succeed.
Their immutable workflow records retain per-job results and downloadable package
artifacts. This report records the independently enumerated local outcomes and
source checkpoints without substituting local Linux tests for native platforms.

The requested code and specification scope is complete. Publication is limited
to `dev`, with unchanged schema-3 storage and no release tag or main merge.
The separate Web client, missing agent tool declaration and synthetic parallel
tool wrapper remain outside this approved repair batch.

## Native CI follow-up: Windows pipe teardown

The report checkpoint `8598de792beb7c5f74e52589dfea64bb2608095e` passed all
nine native quality/platform/package jobs in
[run 43689631894](https://github.com/zqcli/minicore-host/actions/runs/43689631894).
It differs from the implementation pin only in PROJECT.md and this task record.
TUI `0b7d4dc799fbe8c124661713bf8300fc3686ca78` passed all eight jobs in its
linked run, including the four uploaded 0.12.3 platform packages.

The earlier implementation run's Windows job `132735129270` reached its
40-minute limit and was cancelled. The archived log identifies the unchanged
`observation_shutdown` test
`request_eof_before_reader_close_is_clean_and_broken_reader_does_not_cancel_owner`:
it began at 07:21:00 UTC, reported running over 60 seconds at 07:22:00, and was
cancelled at 07:52:38 on 2026-11-15. No assertion failure or panic precedes that
cancellation. The entire transport crate is unchanged from the 0.12.2 baseline.

Independent source review identified a fixture-level wait cycle. On Windows,
dropping a named-pipe reader does not synchronously release every handle retained
by Mio's pending overlapped read. The gated reply could therefore succeed while
the test kept its request pipe open and waited indefinitely for server exit.
The existing `os_pipe::close_reader` helper explicitly disconnects that peer
before dropping it. This follow-up reuses that helper and the existing assertion
for the platform's actual closed-pipe errors; it also bounds previously unbounded
fixture phases with five-second timeouts that identify the stalled phase.

The original two teardown orders, exact unsubscribe response, clean request EOF,
real second Turn completion, retained history and independent owner liveness
assertions remain. No production source, dependency, version, storage format or
specification contract changes. TUI keeps the same immutable Host source pin;
all compiled library source remains identical. The test-only follow-up must pass
the normal nine-job native CI gate before handoff; the earlier cancelled run is
retained as diagnostic evidence rather than reported as successful.

Local follow-up validation passes `cargo fmt --all -- --check`,
`cargo clippy --locked --offline -p minicore-host-transport --all-targets -- -D warnings`,
`cargo test --locked --offline -p minicore-host-transport --test observation_shutdown`
and `git diff --check`. The targeted test passes both native-pipe teardown orders.
An independent worker reviewed the platform helper implementation and the full
test diff; no blocking issue or weakened ownership assertion was found.
