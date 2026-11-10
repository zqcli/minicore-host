# Task104: Static performance analysis and implementation

## Authorization and fixed baseline

The owner requested a new performance task: analyze code and functional costs,
report the findings, then implement with multiple subagents, review, validate and
publish to dev. Analysis does not require another approval before implementation.

- Baseline dev: `4ee5e819798c2b165f9b03cf69dcdfb4a82e2cb4`, version 0.9.20.
- Baseline gate: 956 unique Rust cases, 4 doctests and 28 Python checks.
- SPEC v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Runtime: 0.10.0, locked Git `3aa47629929906832dcb309afb7d242c1ae3e53f`.
- Three analysis/implementation workers were requested with `gpt-6-astra` and
  `max` reasoning. This records requested settings, not a runtime attestation.
- Keep the existing version and dependency pins; this task does not request a
  new release number. Publication is an ordinary history-preserving dev update.

## Static report, before implementation

The selected costs arise from repeated projection, intermediate JSON ownership,
and repeated serialization of growing prefixes. They are observable in the
baseline source. The complexity/allocation conclusions below are static analysis;
they are not measured latency, CPU, memory or throughput improvements.

| ID | Source and actual path | Observed cost | Selected implementation |
| --- | --- | --- | --- |
| P1 | `host-compaction/src/utility.rs::generate_inner` | Each candidate exchange rebuilds, validates and serializes the complete growing utility-request prefix. Similarly sized exchanges produce quadratic preparation work within a chunk. | Count the exact encoded-byte contribution of each exchange once, using an empty request made by the existing constructor as the base; materialize each selected request once. Preserve candidate validation before size rejection, final byte rounding, previous summary, exact chunk boundaries and the oversized first group. |
| P2 | `host-protocol/src/lib.rs::validated_dto!`, `TextRangeData::validate`, `SessionConfig::validate` | Serialization clones every validated DTO field into an owned wire object. Stream deltas repeatedly copy body text; nested configuration objects clone recursively. Some validators clone data merely to read it. | Serialize borrowed wire fields after the same validation; share borrowed validation checks. Preserve the owned deserialize path, strict field rules, omission/null behavior and serializer names. |
| P3 | `host-workspace/src/diff.rs::build_diff` | Every page owns all diff-line strings before selecting the requested byte page, then clones the original line again while constructing fragments. | Keep the real diff and fingerprint computation; traverse borrowed changes and allocate only returned hunk/line fragments. Preserve cursor validation, exact hunk metadata, UTF-8 boundaries and cancellation checks. |
| P4 | `host-storage/src/strict_json.rs` | Root and nested RawValue boxes copy raw subtrees. Decode clones a complete Value tree before decoding and comparing its round trip. | Borrow raw subtrees and deserialize from the retained Value by reference. Still parse all fields, reject duplicates and unsupported/normalized content, and verify the complete round trip. Owned native values continue to need their own data. |
| P5 | `host-core/src/session/query/history.rs::history_read` | A cold page query converts all validated journal entries into public DTOs, then clones only the requested page. Large tool JSON/text is projected outside the requested range. | Validate the full file as before; count public records and project only the requested page. Preserve cursor semantics, revocation, loader coordination, and newer retained-history precedence. |
| P6 | `host-transport/src/operations.rs::run`, `rpc/server.rs` | Every ordinary stdio/WSS result becomes a full Value DOM and is then serialized again into its RPC envelope. | Serialize the typed result directly into a complete response buffer. Preserve dispatch, error codes, the diagnostic completion boundary before queue submission, and send nothing on serialization failure. See the implemented elapsed-interval qualification below. |
| P7 | `host-provider/src/request.rs::encode` | Each model request materializes owned JSON messages/schema trees and embeds them into another JSON value before encoding. Cost repeats with the complete model context. | Use private borrowed request/message wire projections, with owned concatenation only where the provider format requires it. Preserve replay validation, reasoning/tool capability checks, optional fields and actual HTTP payload meaning. |
| P8 | `host-workspace/src/edits.rs::EditJournal::list` | Cursor pages revisit the already validated prefix before applying Turn filtering. | Start iteration after the validated cursor index and retain the same filtering and next-cursor rules. |

JSON object member order may follow typed declaration order after direct encoding;
the contract preserves decoded fields, values and array/event ordering, not
canonical byte ordering of object members. No new public method or DTO is needed.

### Deliberate exclusions

- Keep full-file cold-history validation and save confirmation/fsync ordering.
  A requested page or Turn does not authorize skipping later corruption.
- Keep direct Host -> Agent -> Session -> Runtime ownership, accepted-work
  lifetime, current/saved distinctions, access checks and short state locks.
- Do not introduce caches, persistent indexes, quotas, data truncation, retries,
  fallback recovery, alternate routing or a new benchmark framework.
- Defer broad history/prompt representation changes. Locked Runtime already
  shares Text and ToolCall arguments; a Vec clone is not automatically a deep
  copy of all text/JSON.
- Defer changing tool-output capability authentication to avoid trading a
  demonstrated identity boundary for an unmeasured hot-path hypothesis.
- Defer a streaming journal-reader redesign. It may remove a full-file raw
  buffer, but requires broader file-stamp/hash/offset review than the selected
  codec changes.
- Preserve historical Task090/Task098 harnesses, raw evidence and verifiers
  byte-for-byte. Their old results are not measurements of this task.

## Implementation ownership

| Worker | Isolated worktree | Owned paths |
| --- | --- | --- |
| perf_core | `performance-core` | Core cold-history query and compaction sizing, their focused tests/probes |
| perf_io | `performance-io` | Storage strict JSON codec, workspace diff/edit paging and focused tests/probes |
| perf_transport | `performance-transport` | Protocol borrowed serialization/validation and transport response encoding/tests/probes |
| root | `minicore-host` | Provider encoding, task/project records, integration, measurement, review and publication |

Workers do not publish or modify shared manifests/SPEC. Changes are integrated
only after source review. Shared build resources and measurement windows are
coordinated; isolated source worktrees do not imply isolated machine resources.

## Validation and measurement plan

1. Add or extend focused regressions where behavior can change: exact compaction
   candidate boundaries/error ordering, Unicode/cursor pagination, strict numeric
   and duplicate-field preservation, and JSON shape/error-envelope equivalence.
2. Run relevant existing owner/integration tests, then independent cross-review.
   Keep all existing tests and avoid tests that merely mirror implementation.
3. Use fixed-source release probes against both baseline and optimized code for
   the expensive paths that admit meaningful bounded measurement. Retain every
   sample, build/source/harness hashes, environment and method. Use deterministic
   fixtures only with explicit labeling; separate actual work from artificial
   model/network waits. Do not predeclare gains or performance thresholds.
4. Run a fresh actual Host/stdIO/Runtime/loopback-provider workload with the
   unchanged Task090 harness for functional performance context. It does not
   measure a live provider, TUI rendering, WAN, saturation or capacity.
5. Run the full repository gate and an independent test inventory:

```sh
CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 ./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
PYTHONDONTWRITEBYTECODE=1 python3 tasks/090-fresh-performance/verify_evidence.py
PYTHONDONTWRITEBYTECODE=1 python3 tasks/098-composed-performance/verify_evidence.py
```

The pinned toolchain is added to PATH. Build configuration and any differences
between local resource-saving debug settings and hosted CI are recorded.
Staged and committed whitespace checks cover newly added code, documentation and
the checksum manifest before publication. Final dev head, exact tree, author
identity, clean local sync and hosted CI are verified after the ordinary branch
update.

## Publication scope and withheld evidence

The planned dev publication contains the reviewed optimized code, its tests,
the four probes, measurement runner and auditors, the complete 61-case aggregate
report, and [evidence-manifest.json](evidence-manifest.json). The manifest records
source identities, package member paths, byte counts and checksums; it does not
contain the raw results or logs.

The complete original evidence package remains separately retained. Automatic
approval review rejected public gzip/base64 upload of that package even after
plaintext review. The contents include synthetic fixtures, compiler/test output,
machine paths, PIDs and resource counters; no credentials were identified. That
inspection does not override the automatic review decision. The raw package is
withheld pending explicit owner authorization, so this publication narrows the
disclosed content while preserving the original evidence unchanged.

The archive verifier and summary checker require the separately supplied full
`evidence/` package, including its original `manifest.json`. The public clone's
report and checksum manifest alone cannot recompute the recorded summaries.
Conditional verification commands and expected package paths are documented in
[REPORT.md](REPORT.md).

## Progress and outcomes

- All eight selected changes are implemented. Three requested Astra/max workers
  implemented disjoint scopes and independently reviewed the other scopes; root
  integrated the changes and implemented provider encoding. No blocking source
  findings remain. [Review records](review.json) identify the reviewed commits.
- The exact integrated code checkpoint is
  `dccb62beb676f38e089f1523410d294a3b8d4d82`, tree
  `d4798e4475cae3c78fdef513c19be69881de26dd`. Subsequent task work is reports,
  measurement scripts and retained evidence, without production-code changes.
- Full local gate passed: 964 Rust cases, 4 doctests and 28 Python checks,
  rustfmt, strict Clippy/rustdoc and whitespace. Independent compiled inventory
  reports 126 targets, 124 nonempty. All 956 baseline cases remain, with eight
  focused additions and no original direct test-attribute changes.
- Four paired rounds completed all 240 commands and 61 expanded cases. The
  evidence audit matched semantic dimensions before computing ratios, retained
  every sample and recomputed the complete summary. No statistical-significance
  or universal Host speedup claim follows from these measurements.
- Compaction with 1,024 historical groups of 1 KiB has median-of-round-medians
  prepare + generate time 6,159.14 ms -> 19.39 ms; median paired time reduction
  99.67%. It uses a deterministic immediate summary fixture, not a live model.
  Both versions cover 987 items with one utility call and identical token counts.
- Large provider encoding, protocol serialization, strict JSON and cursor-tail
  edit queries also show scoped gains. Eight cases have a higher median paired
  timing ratio; all cases and observed ranges are retained. Diff remains a
  full-source hash/diff operation and does not demonstrate general speedup.
- Fresh real Host workloads pass in all three runs per version, including real
  tool/process-group cleanup. Results are mixed; sequential sets had different
  system load and deliberate fixture waits. They are descriptive compatibility
  evidence, not proof of general latency, CPU or memory improvement.
- Ordinary successful-operation RPC diagnostics still finish after encoding and
  before queue submission, but their elapsed interval now includes final response
  encoding. The baseline stopped after constructing an intermediate Value.
  Initialization, subscription setup and error branches retain their original timing positions.
  JSON object ordering and
  equivalent native-float exponent spelling may change with typed encoding;
  decoded meaning and array/event ordering are preserved.
- [The complete report](REPORT.md), [validation record](validation.json),
  [checksum-only evidence manifest](evidence-manifest.json) and
  [publication mapping](publication.json) make aggregate results, limitations and
  source identities reviewable within the publication scope above. The version stays
  0.9.20. Publication targets dev with an ordinary history-preserving update;
  hosted CI is checked against the exact published head.
- To prepare sufficient disk space for validation, the previous reproducible
  main-worktree Cargo target was cleaned. Its recorded validation logs and
  published code were preserved.
