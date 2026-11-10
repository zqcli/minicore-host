# Task101: Simplify owner paths and shared facts

## Request, baseline and scope

The owner approved the attached simplification review and asked to implement it
in minicore-host. This follows the existing authorization to integrate reviewed
work into `dev`, preserving remote history and keeping `main` unchanged.

- Baseline: `6ea03b85a318374f3dcb94c38dbe11ac1878f110` on `dev`.
- Working branch: `refactor/owner-path-simplification`.
- SPEC: v4.93, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Runtime: published 0.10.0, revision `3aa47629929906832dcb309afb7d242c1ae3e53f`.
- Preserve all 52 methods, typed/stdio/WSS, direct ownership, independent queues,
  accepted work, save/cleanup boundaries and the R1-R4 corrections.
- Historical result-copy/range changes require separate measurement and are not
  part of this task.

## Implementation

| Area | Change and retained boundary |
| --- | --- |
| Creation | Remove the independent synchronous memory creation implementation; both modes use existing accepted asynchronous owner work. Keep partial receipts and shutdown publication handoff. |
| Metadata | Memory and persistent updates share existing owner acceptance/commit methods. Synchronous memory updates/default changes need no executor. Validation stays outside locks and commit rechecks authority, lifecycle and configuration revision. |
| State | Derive public `saving` from `history_work.running`; a retained failed work item is not an active save. Build persistent record candidates from confirmed `record_base`; retain separate work baseline/candidate. |
| Delegation | Core owns the pure fact types; tools re-exports them and directly passes completions. Serialize only the fact range needed for notifications. Keep capability/permit construction, ports, Weak hooks and lifecycle owners separate. |
| SDK | Three small wrapper macros and a private typed Session request envelope replace mechanical repetition. Public method signatures/docs, diagnostic associations and one call-wait budget stay intact. |
| Protocol | Reuse validated DTO and concrete control receipt templates. Keep strict unknown-field rejection, bidirectional validation and individual receipt invariants. Remove obsolete implementation-stage metadata. |
| Tests | Share real gated Host fixtures, HTTP request/SSE primitives, identical sequential server startup, and transport TLS/pipe/connection mechanics. Preserve scenario assertions, fragmentation/chunked behavior, authentication checks, cancellation witnesses and separate close/error policies. |
| Documentation | Keep active status at the project root; move old checkpoint narratives to a linked historical index. Original task/handoff evidence and the baseline narrative remain retrievable. |

## Intentional Rust API migration

`Host::create_agent` and `Host::create_session` are removed. Direct Rust callers
hold `Arc<Host>` and await `create_agent_async` or `create_session_async` for both
memory and persistent objects. Creation requires the existing live Tokio runtime;
there is no synchronous `block_on` compatibility wrapper. SDK `HostClient` method
names and all transport method names remain unchanged.

The public implementation-stage catalog symbols (`ImplementationStage`,
`planned_stage`, `MethodAvailability`, `MethodCapability`, `staged_catalog`, and
`require_implemented`) are removed. Use the canonical method catalog for names,
access and interaction requirements. The registered `HOST_METHOD_UNAVAILABLE`
Fault and unknown/invalid method handling remain. Synchronous memory config and
default-entry update APIs remain available without a current executor.

## Parallel work and independent review

Three engineering workers were requested with `gpt-6-astra` and `max` reasoning,
as required by AGENTS.md. This records the request, not independently verified
runtime settings. Workers owned disjoint core, protocol/transport and tools paths;
the coordinator handled cross-crate HTTP fixtures, external creation consumers,
documentation and integrated verification.

- Tools worker reviewed core production changes, including accepted creation,
  no-executor metadata commits, saving/failed-work distinction and record templates.
- Core worker reviewed protocol/SDK templates, wire strictness, public signatures,
  diagnostic context and call-wait budgeting.
- Protocol/transport worker reviewed delegation fact reuse and gated Host fixture
  extraction; original scenario/assertion bodies were also compared.
- The coordinator reviews composed changes and the cross-crate integration gate.

These are independent source reviews. Targeted author tests are not represented
as independent reruns or as an external whole-product review.

## Findings and corrections

Independent review found that moving the synchronous persistent Session update
refusal ahead of owner admission would change read-only/closed error precedence.
The refusal now retains the original authority and lifecycle check order. A
focused regression covers this, and another covers memory config/default commits
with neither a current nor retained executor.

The first targeted tools compile caught child-module visibility on the derived
history-saving flag and obsolete imports left by creation removal. The owning
worker corrected these before test execution; the failed compiler log remains in
local validation evidence. No public state or wire contract changed for this fix.

The coordinator's first cross-crate compile found two missing awaits in an
internal resource test that imports the migrated fixture. The first all-workspace
Clippy pass also rejected duplicate inclusions of the new HTTP helper in one
binary. The calls now await the async fixture; sibling fixtures reuse one module
import. No lint was suppressed and no test assertion was removed. The failed
compile/module-check logs remain local; the corrected complete gate follows.

## Verified code and validation

Code checkpoint: [`0a40a5639069e39e3f304aac6fbff079adabd1d2`](https://github.com/zqcli/minicore-host/commit/0a40a5639069e39e3f304aac6fbff079adabd1d2),
tree `dc881a0b356dc0402d40797488ca8685d4bb5e5c`. The implementation and crate-guide content is unchanged from the
passing integrated gate. Subsequent project/report edits only record its results.
The detailed counts, commands and local evidence hashes are in
[validation.json](validation.json).

With pinned Rust 1.90.0, both commands exit 0:

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked --offline -- --list
```

The full gate passes **955 unique Rust cases, 4 doctests and 28 Python checks**,
with no ignored tests or warnings. Whitespace, rustfmt, strict all-target Clippy
and strict rustdoc pass. The inventory has 130 targets, 127 nonempty. Baseline
inventory was 953 cases; the two added cases cover no-executor memory metadata
and synchronous persistent-update authority/lifecycle refusals. Two obsolete
stage-related test names were updated while retaining their applicable receipt
and access assertions. Nested storage subprocess output is not counted twice.

Targeted author gates also passed: core 344 cases, tools 62 cases, and
protocol/transport 237 cases. These overlap the workspace inventory and must not
be added to it. Cross-crate HTTP/resource coverage is included in the passing
workspace gate after the recorded fixture corrections.

Local builds used `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`,
`CARGO_PROFILE_DEV_DEBUG=0` and offline cached pinned dependencies. This omits
incremental files/debug symbols without changing optimization or debug assertions.
Repository profiles, manifests and Cargo.lock are unchanged. Tests exercise real
owners, Runtime, files/processes and loopback transports with named model fixtures;
they are not live remote-model or actual TUI trials. Strict documentation checks
also passed after the complete integration.

## Measured source change

Physical Rust lines are counted using the same file classification as the approved
review. Comments/blank lines count; inline tests in other source files remain in
the non-test-source bucket. This is not pure production SLOC.

| Category | Baseline | After | Added | Deleted | Net |
| --- | ---: | ---: | ---: | ---: | ---: |
| Non-test Rust source files, including their inline tests | 41,601 | 40,663 | 310 | 1,248 | **-938** |
| Explicit Rust test/support files | 71,919 | 71,592 | 909 | 1,236 | **-327** |
| Total Rust | 113,520 | 112,255 | 1,219 | 2,484 | **-1,265** |

Rust file count changes from 326 to 330 because four shared support modules replace
copies. The 12 production crates remain. Explicit core tests grow by 128 lines for
async setup migration and the two meaningful regressions; test/support changes
elsewhere net -455, producing the -327 total. The original test reduction estimate
was a planning range, not a deletion quota. Documentation/validation changes are
counted separately in the manifest and are not included in the Rust reduction.

## Completion and boundaries

The approved creation, metadata, fact-type, SDK/protocol, stage-catalog, repeated
state and fixture simplifications are implemented and reviewed. Owner boundaries,
partial acceptance, the dual queues, current-Turn frozen resources, save/cleanup
work and the R1-R4 fixes remain covered. The code and this report integrate into
`dev` through normal commits without rewriting published history or adding
co-author trailers. Actual published-HEAD CI is checked separately.

Long-history copies/range representations remain the separate measured
performance topic identified by the review. Existing historical performance
reports are retained at their original source checkpoints; this task makes no
latency, throughput or allocation-gain claim.
