# MiniCore Host

## Current work: native multi-platform CI

[Task 105](tasks/105-multiplatform-ci/TASK.md) implements the requested Host CI
and distribution matrix against dev `6ea85fda737635e26b9d7cad17c3df1e54911bd9`.
Runtime is compiled into the Host executable through its existing pinned Cargo
dependency; Host/Runtime interfaces, version 0.9.20 and SPEC remain unchanged.
Linux quality/native and Intel macOS pass the seventh candidate. Windows native
execution exposed a real Job-accounting/process-exit mismatch that remains a
release blocker; its independent native probe and the corrected file-observation
fixtures are recorded in Task 105. Final Apple Silicon, Windows standard-user
and package gates, followed by dev publication, remain pending. Configured
targets and API probes are not treated as passing Host platform acceptance.

## Current performance checkpoint

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

## Current checkpoint: 0.9.20

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
- Host/Agent/Session lifecycle, defaults, immutable Turn configuration, controls, deletion, graceful/force shutdown, ownership and reclamation; real Runtime dual queues and same-Turn continuation, authoritative complete/partial history, independent save/cleanup failures.
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

- Complete SPEC v4.93 is unchanged, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- All Runtime edges/lock pin published 0.10.0 `3aa47629929906832dcb309afb7d242c1ae3e53f` on `feat/host-history-handoff`. [Publication evidence](docs/runtime-retry-wait-publication.json) and exact-head CI42746444546 passed; producer and independent Runtime gates pass 324 cases and 2 doctests. There is no local override/bootstrap/vendor path. Actual elapsed retry waits appear in joined reports and Host diagnostics in integer milliseconds, with no added client event field or retry-policy change.
- Runtime upstream dev 3fd5ecda, reference Agent f36a87366d8e2fe0a1c8216463887902e471c739 and TUI 3beeb1d8a525e58ce6352e05061f242c5580e248 remain unchanged. Agent is reference, not baseline; Agent/TUI still pin old Runtime b276d9c. No joint TUI integration is claimed.

| Stage | Actual boundary | Remaining |
| --- | --- | --- |
| 1. Contracts/storage | Implemented; real files, locks, corruption, saved-result and save-barrier tests | External whole-contract review; no new unpublished-creation recovery API |
| 2. Host/entities | Implemented; ownership, config/defaults, shutdown/delete/reclaim and diagnostics exercised | External final review |
| 3. Session/Runtime | Implemented; real execution, queues, history, tools, approval, compaction and delegation exercised | Real authorized external-provider trial; external final review |
| 4. Client access | Typed/stdio/authenticated WSS, operation rights/reload, observation and queries exercised | Actual consuming TUI acceptance; external final review |
| 5. Actual TUI | Intentionally unchanged this round | Explicitly deferred, not passed |
| 6. Convergence | Scoped reviews, complete fixed gates, real performance evidence and incremental dev backups | External final acceptance and main decision; not automatically authorized by green CI |

Local-use limits remain explicit: uncontrolled destruction is not normal stopped shutdown; unpublished-create save failure has no new recovery endpoint; workspace Git checks assume stable local configuration/attributes; Linux is the tested platform. No speculative defensive platform was added.

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
