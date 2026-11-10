# Task100: Resolve the first two backend review batches

## Scope and fixed baseline

The owner requested implementation, review, a report and publication to `dev` for
the first two batches of the independent backend review: R1/R2 (P1) and R3/R4
(P2). The starting remote `dev` is
`628c17312149d76ab1d3ecc84bdb1723e6d6309a`; its backend code matches the prior
`be0e99552d283289fb1c0cc13915874ce0ba757f` checkpoint. The original diagnostics and
their actual failing/passing assertions were recovered from the previous review
evidence, rather than treating a green diagnostic assertion as correct behavior.

SPEC v4.93 remains unchanged, SHA-256
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Runtime remains the fixed published 0.10.0 revision
`3aa47629929906832dcb309afb7d242c1ae3e53f`.

## Acceptance requirements

- R1: A Session published concurrently with accepted Host shutdown receives the
  current Agent shutdown phase before its creation finishes. Both publication
  orders retain the real cleanup and persistent-root release barriers.
- R2: A repeated upstream ToolCallId within one public Turn is rejected before it
  replaces a prior invocation or becomes accepted complete history. The failure
  is a model protocol failure; valid prior results and durable bytes remain
  readable, and the Turn and Host shutdown can settle. Separate public Turns may
  reuse the same upstream ID. No ID rewriting or storage-validation weakening.
- R3: `agent inspect` with complete Turn output reads the existing retained/saved
  result path, including after persistent restart, without inventing execution
  metadata for old saved results.
- R4: Public input history preserves actual message identity, source, mode and
  reply association through live, retained-result, saved-result and cold-history
  reads. Reuse existing provenance and journal records; do not restore old queues
  or create an independent message database.

## Implemented changes

R1 captures `host_shutdown_phase` in the same Agent lock boundary that inserts the
Session. The Session receives that phase outside the Agent lock while the
accepted creation count still holds the shutdown barrier. Publication before the
scan is included by the scan; publication after the scan inherits its phase.
Existing release guards and the original creation/save owners remain intact.

R2 binds a narrow `TurnModel` once per public Turn, across its internal Loops.
Before forwarding a `ToolCallStart`, it checks the existing accepted ToolRecord
map for that Turn. A repeated raw ID becomes a nonretryable Runtime
`InvalidProviderResponse` / `ModelMalformedResponse` before response assembly.
The public existing error registry reports 4299 with a specific repeated-ID
message. There is no new ID index, ID rewriting or relaxed journal validation.
Valid earlier stream content remains an unexecuted partial; the invalid call
cannot enter accepted history or be recovered from a report's unseen suffix.

R3 adds the existing asynchronous result snapshot to the trusted Turn capability.
Full Turn inspect output uses that path before any current-run `turn_info` query.
Held results retain actual state and LoadId; saved-only results expose their saved
terminal/content without fabricated state, configuration or revisions. Default
lightweight and explicit-range inspection retain their current execution paths.

R4 adds optional `message_id`, `source`, `mode` and `in_reply_to` to public User
history. Live and saved reads project actual Input entries through the same
conversion. Held results retain only their original native starting position and
use the existing InputProvenance map. Compaction retains append-only native
history; reload reconstructs the same positions; reclamation drops retained
results and provenance together. Unknown native-only metadata stays absent, and
the model prompt's existing metadata encoding is unchanged.

The final validated source is `094d261d42449fc0612a5755763f22c086332967`, tree
`b932ddd3c3579eff1c8080544b6d24c7a4f44cae`. Its production fixes already compose at
`70082e4e5c66cc54fad6d88152bfcb5e1644b386`; the two following commits only correct
test setup. Compared with the review baseline, 11 production Rust files change
by +239/-34 text lines, 9 explicit test files by +1494/-92, and crate guides by
+13/-1. These counts include comments/blank lines; they are not executable SLOC.
No dependency, manifest, lockfile or SPEC change.

## Targeted verification and independent review

Workers were requested with `gpt-6-astra` and `xhigh` reasoning, following the
owner's stated preference; effective runtime settings are not independently
attested. Each fix was reviewed by a worker other than its author. These reviews
read exact commits and original test logs; they do not claim separate reruns of
the author's passing tests. All four reviews found no blocking issue.

| Finding | Permanent regression and observed result |
| --- | --- |
| R1 | Four added publication/shutdown cases; `host_shutdown` passes 14 tests. Temporarily restoring only baseline production code makes the original force/default-save case fail with the actual missing shutdown fence. |
| R2 | Six new identity cases plus existing `observation_tools` and `history_partials` pass 9 tests. Baseline memory overwrites the original response index; baseline persistent reaches the actual duplicate-identity save fault and cannot finish its wait. |
| R3 | Real Runtime AgentTool saved-result read after graceful restart, held result, and active lightweight/range reads pass. The original three-case target passes before R4 composition. |
| R4 | Five combined inspect/history tests and 111 core unit cases pass. Regressions cover identical bodies, distinct IDs, reply/mode, later Turns, unload/reload, cold/saved results, actual parent/child inputs and completion notifications, and unknown legacy metadata. Existing saved-result restart (4) and tool Host integration (9) also pass; this is 129 distinct targeted cases, not 134 after a repeated five-case subset. |

All four independent review records and the integration-test investigation are
in [`evidence`](evidence). Original red/green logs were inspected and are retained
locally; log names in those records refer to this local evidence, not committed
files. Red logs are deliberate baseline failures, not failures of the fixed
checkpoint. Models are explicitly scripted fixtures; Host, Runtime, tool
execution and persistent files are real.

## Findings during integration

The first full gate stopped at an existing observation equality test: the two
experiments generated different MessageIds, which R4 correctly made visible.
The test now clones the same original MessageInput into its independent Hosts.
The complete history equality, live/cold equality and no-execution-on-read
assertions remain. The targeted observation suite passes both tests.

The second gate was interrupted with exit 130 after an existing adapters test
waited indefinitely for its accepted-creation gate. Its 30 ms AgentCreate call
deadline also covered the first, pre-admission validation. A controlled fixture
proved that this deadline can expire with no receipt or Agent, so the second
validation never occurs. The original hung process's stack was not captured;
the retained investigation distinguishes that limit from the controlled proof.

The corrected test verifies AgentCreate abandonment/disconnection after actual
acceptance, and verifies the real 30 ms SDK timeout with SessionCreate, whose
owner accepts before its first await. Both transports retain the original
completion/unique-object/one-model-call checks; stage waits are bounded and a new
pre-admission case checks the absence of a receipt and Agent. No production
logic, sleep or increased 30 ms deadline was introduced. All 13 adapters cases
pass, and an independent source review approves the correction.

## Final validation and published code objects

At the final source above, these commands both exit 0:

```sh
CARGO_BUILD_JOBS=2 ./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
```

The gate passes **953 unique Rust cases, 4 doctests and 28 Python checks**, plus
rustfmt, strict all-target Clippy, strict rustdoc and full-history whitespace.
The inventory contains 127 nonempty test binaries out of 130 listed targets.
There are 14 additional unique Rust cases over the 939-case baseline. Nested
storage subprocess output is not counted as another test case.

| Change | Integrated source | Remote object |
| --- | --- | --- |
| R1 publication/shutdown handoff | `b316b1a8a2bd0c9770efb79009876965fb29ec60` | [6fd6188](https://github.com/zqcli/minicore-host/commit/6fd6188b99468e61768099d46bee4f875e838059) |
| R3 saved Turn inspect | `0ea9c133b3d31664d727382fd5806d78a051c9a8` | [0a5a0bd](https://github.com/zqcli/minicore-host/commit/0a5a0bd3f35a880c460f76edca24040218d1f189) |
| R2 public Turn tool identity | `7a4e14b8af42698bcb3853d43dbc54d68a2025f5` | [074558a](https://github.com/zqcli/minicore-host/commit/074558ad78c5c983ada608ab3a6cb7a5f539a544) |
| R4 public input provenance | `70082e4e5c66cc54fad6d88152bfcb5e1644b386` | [9f0c70e](https://github.com/zqcli/minicore-host/commit/9f0c70e50632f8a5bcf82640c32ef59e97c81d08) |
| Identical observation fixture input | `ac503cd7b4ba85a62594f34f10120f0dd7b60752` | [0722ddf](https://github.com/zqcli/minicore-host/commit/0722ddfc2e0754b4f2cf82390ecaf628aa5b4ebb) |
| Actual creation acceptance in tests | `094d261d42449fc0612a5755763f22c086332967` | [564a486](https://github.com/zqcli/minicore-host/commit/564a4865cbd979d6dab61498c52453e322ef2aad) |

Each remote tree matches its exact integrated source. Parents preserve the
existing `dev` history; authors and committers are zqcli with scoped English
subjects and no coauthor trailers. [The validation manifest](validation.json)
records mappings, checksums, counts, commands and limits. This report accompanies
the six code/test commits in one fast-forward
`dev` publication. The actual report HEAD and its hosted CI are checked after
publication and reported to the owner, not predicted inside their own commit.

## Publication evidence boundary

Automatic approval review rejected uploading one compressed raw test log because
it could contain local sensitive information and raw-log publication was not
explicitly authorized. No retry or alternate upload route was used. All original
test logs are excluded from the published commit; the report instead includes
reviewed results, reproduction commands and the six checked textual review and
investigation records. The manifest identifies the locally retained logs by
name and checksum without publishing their contents. This documentation-only
adjustment does not change the validated code or test trees.

## Boundaries

The third and fourth review batches (actual consuming TUI, authorized external
provider acceptance, and targeted long-history/performance work) are not part of
this task. The TUI and other repositories are unchanged. No main merge or
force-push is authorized or planned. Green tests close the demonstrated scope;
they do not establish complete v4.93 product acceptance or capacity guarantees.
