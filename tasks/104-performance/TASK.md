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
| P6 | `host-transport/src/operations.rs::run`, `rpc/server.rs` | Every ordinary stdio/WSS result becomes a full Value DOM and is then serialized again into its RPC envelope. | Serialize the typed result directly into a complete response buffer. Preserve dispatch, error codes, diagnostic completion timing and send nothing on serialization failure. |
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
Staged and committed whitespace checks cover newly added evidence files before
publication. Final dev head, exact tree, author identity, clean local sync and
hosted CI are verified after the ordinary branch update.

## Progress and outcomes

- Static analysis completed on the fixed baseline; selected scope above.
- Implementation, independent review, measurements and final gate are pending.
- To prepare sufficient disk space for validation, the previous reproducible
  main-worktree Cargo target was cleaned. Its recorded validation logs and
  published code were preserved.
