# MiniCore Host

## Current checkpoint: provider decoding and assembly (Task 108)

[Task 108](tasks/108-provider-assembly/TASK.md) implements the approved review
item 5 against dev `913b736bd4e3d72ef5f9a9a7d9d0b381925a6a7a`: directly decode
ordinary provider frames and make Runtime the single owner of raw tool argument
assembly and parsing. The SPEC clarifies original argument preservation,
protocol metadata in partial results and unchanged execution/retry boundaries.
Host remains 0.10.1; external dependencies and the durable format are unchanged.
The full local gate passes 1,126 unique Rust cases, 6 doctests and 28 Python
checks, formatting and strict Clippy/rustdoc. Publication to dev precedes the
requested six-pair performance recheck using the committed measurement source.

## Previous checkpoint: execution overhead (Task 107)

[Task 107](tasks/107-execution-overhead/TASK.md) implements the owner's approved
review items 1–4: lighter tool-output capabilities, one model-output buffer,
shared frozen tool declarations, and less repeated compaction work. Its exact
baseline is dev `5911d56a64bb2302e075b11cd44d667643999f07`. Host 0.10.1,
SPEC 4.10.1 and the provider decoder were unchanged in that task. Its published
implementation is dev `913b736`, the source baseline of Task 108.

## Previous checkpoint: integrated execution layer (0.10.1)

[Task 106](tasks/106-host-runtime/TASK.md) implements the owner's approved
SPEC 4.10.1 design: Runtime becomes an internal Host workspace package,
Session drives each complete public Turn, and ordinary and compaction model
requests share the same execution implementation. The source checkpoints are
Host `685806af4761eaaa75bae11cd70e5d7a7a0177b0` and its actual Runtime dependency
`3aa47629929906832dcb309afb7d242c1ae3e53f`.

The implementation is published on dev as
[`1e198a8`](https://github.com/zqcli/minicore-host/commit/1e198a8af2643c64ac4497ac006834adfe65f50d),
with the exact tree of the locally verified source `6b2f217`. All 13 workspace
packages use 0.10.1. The local full gate passes **1,110 unique Rust cases,
6 doctests and 28 Python checks**, formatting, strict Clippy/rustdoc and whitespace.
The Linux release package contains one executable and passes smoke/readback checks.
All 225 external locked package versions, sources and checksums are unchanged.

[Native CI run 43242142126](https://github.com/zqcli/minicore-host/actions/runs/43242142126)
passed all nine jobs on its first attempt: quality, native tests on Linux x64,
macOS ARM64/x64 and Windows x64, and all four native packages. Each package was
downloaded and independently checked against its archive and executable hashes;
each contains exactly 19 files and one Host executable. The
[CI record](tasks/106-host-runtime/ci-source.json) retains the exact evidence.
The remote still has only dev;
publication used a leased fast-forward and preserved its prior history.
The requested `v0.10.1` tag awaits an authenticated Git push: the available
connector can update branches but cannot create tags, and terminal Git has no
push authentication. This verification record does not attest a remote tag.

[Performance evidence](tasks/106-host-runtime/performance/README.md) retains
the initial three paired rounds and a balanced four-round confirmation. The
confirmation's ordinary-request medians are shorter, while compaction medians
increase by about 0.29 ms and 1.38 ms for its 256/1,024-group fixtures. Both sets,
including their opposite results and order effects, remain available. Source
review found no new history-size traversal or deep copy in the inspected paths,
but did not establish a cause for that slowdown signal. This checkpoint does not
claim general acceleration or proven absence of regressions.

[Task 106](tasks/106-host-runtime/TASK.md),
[validation](tasks/106-host-runtime/validation.json) and
[publication mapping](tasks/106-host-runtime/publication.json) record the exact
source identities, commands, outcomes, limitations and remaining tag step.

## Previous checkpoint: native multi-platform CI (0.9.20)

[Task 105](tasks/105-multiplatform-ci/TASK.md) completes native Linux x64, macOS
ARM64, macOS x64 and Windows x64 CI and gated release packaging against dev
`6ea85fda737635e26b9d7cad17c3df1e54911bd9`. The verified code is
[`f29d3c9c`](https://github.com/zqcli/minicore-host/commit/f29d3c9c7b7d2f10ed6bcc72a131ca3d85cd37a6),
tree `a8d5817c9fef4e2802340b396588e481eea3ba41`.
[Run 43068086506](https://github.com/zqcli/minicore-host/actions/runs/43068086506) passes all nine jobs:
Linux quality, four native test jobs and four native release packages.

| Native platform | Unique Rust cases | Doctests | Release package |
| --- | ---: | ---: | --- |
| Linux x64 | 967 | 4 | tar.gz, verified and uploaded |
| macOS ARM64 | 960 | 4 | tar.gz, verified and uploaded |
| macOS x64 | 960 | 4 | tar.gz, verified and uploaded |
| Windows x64 | 948 | 4 | ZIP, verified and uploaded |

Each native listing has 127 targets. Counts exclude nested subprocess output
and are not summed across platforms. Linux's full gate also passes 28 Python
checks, formatting, strict Clippy/rustdoc and whitespace. Windows additionally
passes five existing storage/Host contracts under an actual ordinary user;
its profile, restricted executable access and account cleanup are verified.

Every package contains one Host executable, configuration and documentation.
The actual staged binary runs successfully before archive readback verifies
the exact file set, executable SHA-256 and Unix permissions. Runtime 0.10.0
at `3aa47629929906832dcb309afb7d242c1ae3e53f` is compiled into Host 0.9.20 through
the existing pinned Cargo dependency. SPEC, Host/Runtime request handoff and
Turn/Step/compaction ownership remain unchanged.

Independent final review found no remaining P1/P2 findings in this scope.
Published dev `282a678b0ab7ee94ba774451619f640a62d9a050` also passes all nine
jobs in [run 43071596301](https://github.com/zqcli/minicore-host/actions/runs/43071596301),
including four uploaded native packages. The owner's 2026-11-12 branch
integration request joins the two completed Windows investigation histories
into dev, retaining the verified implementation and recording the explicit
resolution in Task 105. The main and multi-platform implementation branch tips
were already ancestors of dev. The integration changes only these project/task
records; production, tests, dependencies, workflows and scripts retain their
verified blobs. The resulting merge commit has its own automatic CI run.
Exact jobs, artifact URLs and distinct binary/archive/artifact hashes are in
[the validation record](tasks/105-multiplatform-ci/validation.json). See
[the CI guide](docs/ci.md) for downloads, local NTFS requirements and optional
Git Bash prerequisites. This task does not change versions or create a tag.

## Included performance checkpoint

[Task104](tasks/104-performance/TASK.md) implements eight selected optimizations
against dev `4ee5e819`: compaction request sizing, protocol/provider/RPC encoding,
strict JSON decoding and history/workspace paging. Three requested Astra/max
workers implemented disjoint scopes and cross-reviewed the integrated changes.
Version 0.9.20, SPEC, dependency pins and historical evidence are preserved.

[The performance report](tasks/104-performance/REPORT.md) records all 61 cases
from four paired release-probe rounds (240 commands), including regressions and
measurement limits. The deterministic 1,024-group compaction fixture changes
prepare + generate median time from 6,159.14 ms to 19.39 ms, with a median paired
reduction of 99.67%; this excludes live-model inference. No general Host speedup
is claimed. Fresh real-Host scenarios pass in three runs per version, with mixed
timings and materially different system load.

Code checkpoint `dccb62be` passes 964 Rust cases, 4 doctests and 28 Python checks,
plus formatting, strict Clippy/rustdoc and whitespace. All 956 baseline cases
remain. [Validation](tasks/104-performance/validation.json) and
[publication mapping](tasks/104-performance/publication.json) retain exact
source/tree identities. The planned publication contains the reviewed optimized
code, tests, probes, measurement runner and auditors, all 61 aggregate results,
and a [checksum-only evidence manifest](tasks/104-performance/evidence-manifest.json).

The raw evidence package is retained separately and withheld from publication
after automatic approval review rejected public upload of opaque gzip/base64
payloads. Its reviewed contents contain synthetic fixtures, compiler/test output,
machine paths, PIDs and resource counters; no credentials were identified. The
automatic review decision remains binding, and raw-evidence publication awaits
explicit owner authorization. Recomputing the recorded summaries requires the
separately supplied full `evidence/` package; the public report and manifest alone
do not supply those inputs.

## Organization checkpoint: 0.9.20

[Task103](tasks/103-project-organization/TASK.md) implements the approved project
organization review. Direct owners and public crate-root APIs are preserved;
Session execution/loading/closing/query modules, transport adapters, resource
composition, tool projections and storage validation now follow their existing
responsibilities. Shared test support is no longer owned by an old task directory.

All 12 Host packages inherit **0.9.20** from the workspace. The 34 internal
requirements retain their original constraint semantics; external dependency
revisions and features are unchanged. The GitHub release source is
[`7152b1be`](https://github.com/zqcli/minicore-host/commit/7152b1be236497630d79e96d64dc57316c8040db),
with the identical tree to the validated local source `fa2c4004`.
[Publication mapping](tasks/103-project-organization/publication.json) records
all actual GitHub identities and preserves the original remote ancestry. The
owner explicitly authorized this `dev` publication.

Three independently assigned Astra/max workers implemented disjoint scopes and
cross-reviewed the other scopes, with no blocking findings. These are requested
model settings, not an independent runtime attestation. The integrated release
gate passes **956 unique Rust cases, 4 doctests and 28 Python checks**, formatting,
strict Clippy/rustdoc and whitespace. The authoritative listing has 126 targets,
including 114 integration test binaries (four fewer after owner diagnostic grouping).
The sole added case checks real-owner message/ToolCall inspect projections.

[Validation](tasks/103-project-organization/validation.json) records the exact
commands, source/tree, counting method, log hashes, dependency comparison and
unchanged historical performance verification. Rust physical lines change by
+448 in non-test-source files and +287 in explicit tests/support. This release
improves responsibility boundaries and type projections; it claims no measured
performance increase. The 12-crate layout, trusted embedding hooks and frozen
performance harnesses remain deliberately intact.

## Included owner-path simplification

[Task101](tasks/101-owner-path-simplification/TASK.md) implements the approved
simplification review on top of `dev`
[`6ea03b85a318374f3dcb94c38dbe11ac1878f110`](https://github.com/zqcli/minicore-host/commit/6ea03b85a318374f3dcb94c38dbe11ac1878f110).
Local code checkpoint `0a40a563`, published with the identical tree as
[`4ba0afa0`](https://github.com/zqcli/minicore-host/commit/4ba0afa0ad1d2022195be1a11a08cbacb19976ff),
passes the complete gate: **955 unique Rust cases, 4 doctests and 28 Python checks**,
strict Clippy/rustdoc, rustfmt and whitespace. The fixed baseline contains 953
cases. Rust source/test lines net **-1,265** (938 non-test-source, 327 explicit
test/support); [the manifest](tasks/101-owner-path-simplification/validation.json)
records the exact accounting and local validation boundaries.

The change unifies creation and owner metadata commits, reuses delegation facts
and protocol/SDK templates, derives history-saving state from the actual save
work, and shares duplicated integration fixtures. All 52 canonical methods and
typed/stdio/WSS access stay in scope. Synchronous Rust creation conveniences and
implementation-stage catalog APIs are intentionally removed; migration details
are recorded in Task101 and the core/protocol crate guides.

The preceding [Task100 review fixes](tasks/100-review-r1-r4/TASK.md) remain part of
the baseline: creation/shutdown phase handoff, public-Turn ToolCallId rejection,
saved-result `agent inspect`, and actual input provenance. The simplification
preserved SPEC v4.93, dependency pins and the lockfile; the 0.9.10 version bump
updates only the 12 Host package versions in that lockfile. Long-history
representation and measured performance changes are separate work.

## Implemented scope and closed findings

- Strict protocol/Fault/receipt contracts, direct identities, real persistent/memory storage, OS ownership locks, full-history validation and original-save confirmation.
- Host/Agent/Session lifecycle, defaults, immutable Turn configuration, controls, deletion, graceful/force shutdown, ownership and reclamation; Session-owned input queues and complete Turn execution, internal Runtime model/tool calls, authoritative complete/partial history, independent save/cleanup failures.
- Production HTTP/SSE resources; builtin agent/read/edit/write/bash, frozen Ask/Auto/ReadOnly approvals, real process supervision and file/edit ownership, manual/automatic compaction and independent delegation/notification scopes.
- All 52 catalogue routes have real shared typed/stdio/WSS implementations, including authentication/operator rights, reload, observations/UTF-8 catch-up and actual retained or saved result reads. Auxiliary discovery/workspace queries deliberately retain their basic specified scope, not a generic platform.
- Complete reconstructed business diagnostic scope is fixed in [Task053](tasks/053-business-diagnostics/TASK.md). Runtime actual retry waits, first force claims, rejected resource/message attempts and true failure/save-recovery scheduling are source-derived facts, not observer reconstruction or acknowledgements.

The original saved-result restart P1 and invalid-wire P2 are closed by [Task091](tasks/091-saved-result-review/TASK.md). The automatic-compaction 3702→4999 replacement is closed by [Task092](tasks/092-compaction-review/TASK.md), with original utility and independent persistence Faults retained. [Task093](tasks/093-session-handoff-cleanup-review/TASK.md) gives separate scoped approvals for all final diagnostic slices. [Task094](tasks/094-convergence-audit/TASK.md) reconciles the historical 52-method audit without claiming method count proves all behavior. [Task095](tasks/095-saved-history-diagnostics/TASK.md) confirms saved-read errors were already correctly logged; no redundant production hook was added. No demonstrated production finding remains open in these scoped reviews.

## Running and performance evidence

Use the [executable guide](crates/host-app/README.md) and
[tool approval guide](docs/tool-approval.md). The process uses protocol-only
stdout; builtins are agent/read/edit/write/bash. `apply_patch` is not implemented
or required by name in SPEC. The README configuration smoke and its limits remain
recorded in [Task097](tasks/097-usage-docs/TASK.md).

The retained measurement entry is `scripts/task090_perf.py`; `task058_perf.py`
is its frozen predecessor. Their shared-looking code is intentionally retained
byte-for-byte: Task090 and Task098 validate the original harness hashes. New
measurements need their own source/evidence record. Organization changes do not
rewrite those hashes or turn old measurements into current performance claims.

The existing [c22d8c8 measurements](docs/performance-c22d8c8.md) and
[older f36761a report](docs/performance-f36761a.md) describe their fixed historical
sources. Their raw evidence and recomputation scripts remain available. They are
not measurements of this simplification or evidence of a performance increase.

## Dependencies and six-stage boundary

- The current complete design baseline is SPEC v4.10.1, SHA-256 `b1426061eb63478f392e8f5638792df1c7d8a1bad08d80c80f3203288816182b`.
- Runtime is the internal `minicore-host-runtime` 0.10.1 package at `crates/host-runtime`, referenced through a workspace path dependency. Its imported source was external Runtime 0.10.0 `3aa47629929906832dcb309afb7d242c1ae3e53f`; Host no longer builds against that Git dependency. The standalone loop/report lifecycle is removed. The shared model driver returns authoritative output and actual retry waits, while Session owns the Turn and history. The prior external-source [publication evidence](docs/runtime-retry-wait-publication.json) remains historical evidence.
- The original Runtime repository, reference Agent baseline `f36a87366d8e2fe0a1c8216463887902e471c739`, and TUI baseline `3beeb1d8a525e58ce6352e05061f242c5580e248` are outside this change. Agent remains a code reference. No joint TUI integration is claimed.

| Stage | Actual boundary | Remaining |
| --- | --- | --- |
| 1. Contracts/storage | Implemented; real files, locks, corruption, saved-result and save-barrier tests | External whole-contract review; no new unpublished-creation recovery API |
| 2. Host/entities | Implemented; ownership, config/defaults, shutdown/delete/reclaim and diagnostics exercised | External final review |
| 3. Session/Runtime | Implemented; real execution, queues, history, tools, approval, compaction and delegation exercised | Real authorized external-provider trial; external final review |
| 4. Client access | Typed/stdio/authenticated WSS, operation rights/reload, observation and queries exercised | Actual consuming TUI acceptance; external final review |
| 5. Actual TUI | Intentionally unchanged this round | Explicitly deferred, not passed |
| 6. Convergence | Scoped reviews, complete fixed gates, real performance evidence and incremental dev backups | External final acceptance and main decision; not automatically authorized by green CI |

Local-use limits remain explicit: uncontrolled destruction is not normal stopped shutdown; unpublished-create save failure has no new recovery endpoint; workspace Git checks assume stable local configuration/attributes. Task 106 verifies this source on the four native runners established by Task 105; filesystem and local-tool requirements remain documented in [the CI guide](docs/ci.md). No speculative defensive platform was added.

## Historical review index

Detailed findings, failed attempts, exact source mappings and test evidence stay
in their original task/handoff records. Earlier status statements describe those
checkpoints, not the current open-work list.

| Checkpoint or topic | Preserved record |
| --- | --- |
| R1-R4 fixes; code `564a4865`, documentation `6ea03b85`; 953 Rust cases | [Task100](tasks/100-review-r1-r4/TASK.md), [validation manifest](tasks/100-review-r1-r4/validation.json) |
| Backend review `be0e9955`; 939 Rust cases; publication/source mapping | [Task099](tasks/099-backend-review-handoff/TASK.md), [fixed handoff](docs/recovery-checkpoint-review.md), [publication map](docs/recovery-publication.json), [validation](docs/backend-review-validation.json) |
| Final diagnostic integration and contract audit | [Task053](tasks/053-business-diagnostics/TASK.md), [Task091](tasks/091-saved-result-review/TASK.md), [Task092](tasks/092-compaction-review/TASK.md), [Task093](tasks/093-session-handoff-cleanup-review/TASK.md), [Task094](tasks/094-convergence-audit/TASK.md), [Task095](tasks/095-saved-history-diagnostics/TASK.md), [Task096](tasks/096-convergence-checkpoint/TASK.md) |
| Executable, query, saved-history and input-provenance checkpoints | [Executable handoff](docs/executable-review-handoff.md), [historical method audit](docs/current-contract-audit.md) |
| Execution integration; original 650-case checkpoint and lease correction | [Task041](tasks/041-execution-integration/TASK.md), [execution handoff](docs/execution-review-handoff.md), [linear source map](docs/execution-linear-map.json) |
| Owner/resource integration `c6d8a5e1`; 506 Rust cases | [Task029](tasks/029-owner-resource-integration/TASK.md), [integration handoff](docs/integration-review-handoff.md), [publication map](docs/owner-integration-publication.json) |
| Tool cleanup, terminal claim and root-lease findings | [Task033](tasks/033-turn-tool-cleanup/TASK.md), [Task034](tasks/034-history-lock-reopen/TASK.md) |
| Initial foundation `82583b31`; 340 Rust cases | [Task028](tasks/028-host-publication/TASK.md), [foundation handoff](docs/review-handoff.md), [publication map](docs/host-publication.json) |
| Workspace replacement and unavailable old unpublished evidence | [Task083](tasks/083-second-workspace-recovery/TASK.md) |
| Built-in tool scope | [Task056](tasks/056-built-in-tool-scope/TASK.md) |

The complete earlier project narrative remains in
[the fixed baseline](https://github.com/zqcli/minicore-host/blob/6ea03b85a318374f3dcb94c38dbe11ac1878f110/PROJECT.md).
Historical evidence is preserved; lost unpublished objects are not claimed as
recovered. Development integrates into `dev` with ordinary history-preserving
updates. External whole-product review, actual TUI/external-provider acceptance,
and a subsequent main-branch decision remain separate work.
