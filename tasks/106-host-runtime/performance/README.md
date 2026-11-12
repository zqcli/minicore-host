# Task 106 focused performance checks

The balanced confirmation has a compaction regression signal: candidate pooled
medians are **5.9% longer at 256 groups and 7.3% longer at 1,024 groups**. The
ordinary Host transitions have shorter confirmation medians, but individual
rounds and run order vary. These results do **not** establish overall acceleration
or the absence of regressions. The cause of the compaction signal remains
unattributed; this round did not collect a CPU profile.

The baseline is Host `685806af4761eaaa75bae11cd70e5d7a7a0177b0` and its pinned
Runtime `3aa47629929906832dcb309afb7d242c1ae3e53f`. The measured candidate is Host
`1009c719ced338ebb05c5bacccc8c9403f882ec2`, including the optional outer-deadline
correction. Its source-identical published commit is
`982593255fcf9b31ef1cd30a1b668420120e2ac6`, tree
`5e1433f2c9fb874da7feb4fdec78388de6c8c514`. Earlier `b55b4f3` builds are not
included. Both measured variants use Rust 1.90.0 release builds and the same
workloads and assertions.

Both runs completed on 2026-11-12 in separately coordinated windows without
project builds or tests. The initial three rounds used AB/BA/AB, where A is
baseline and B is candidate. Its 256-group increase and strongly order-dependent
1,024-group result prompted one additional four-round
AB/BA/AB/BA confirmation. The script, sources, executable hashes, helper and
normalized compaction probe were identical across the two runs. No measured
sample or inner repetition was removed, and no further timing was performed.

All values below are pooled medians in milliseconds. Each Host sample is one
response-to-request transition; each compaction sample is one prepare+generate
repetition. Positive changes mean longer candidate durations.

| Case | Initial: baseline → candidate | Change | Confirmation: baseline → candidate | Change | Samples per variant, initial / confirmation |
| --- | ---: | ---: | ---: | ---: | ---: |
| Same-Turn steering, 16 requests | 1.171 → 1.202 | +2.6% | 1.146 → 0.982 | -14.3% | 45 / 60 |
| 64 historical Turns, then 8 requests | 3.128 → 3.037 | -2.9% | 2.952 → 2.436 | -17.5% | 21 / 28 |
| Compaction, 256 groups | 6.489 → 7.077 | +9.1% | 4.885 → 5.175 | +5.9% | 9 / 12 |
| Compaction, 1,024 groups | 37.731 → 19.388 | -48.6% | 18.976 → 20.357 | +7.3% | 9 / 12 |

The initial 1,024-group apparent gain is not a reliable benefit: its two
baseline-first rounds were much slower than its candidate-first round, and the
balanced confirmation changed direction. In the confirmation, both compaction
sizes were slower in three of four paired rounds. This is a regression signal in
this fixture, not a proven causal cost of the shared model driver. The ordinary
Host cases also retain mixed individual-round results.

The following ratios are candidate/baseline; values below 1 mean shorter measured
durations. Round ratios compare each round's medians. AB/BA columns pool only the
raw samples from that order within the indicated dataset.

| Case | Dataset | Ratios in round order | AB pooled ratio | BA pooled ratio |
| --- | --- | --- | ---: | ---: |
| Same-Turn steering | Initial | 0.745 / 1.229 / 0.902 | 0.910 | 1.229 |
| Same-Turn steering | Confirmation | 0.635 / 1.236 / 0.782 / 0.880 | 0.694 | 0.992 |
| Large history steering | Initial | 0.838 / 1.024 / 1.292 | 0.981 | 1.024 |
| Large history steering | Confirmation | 0.791 / 0.783 / 1.208 / 0.766 | 0.985 | 0.786 |
| Compaction, 256 groups | Initial | 0.748 / 0.483 / 2.315 | 1.259 | 0.483 |
| Compaction, 256 groups | Confirmation | 1.115 / 1.071 / 1.048 / 0.855 | 1.089 | 0.949 |
| Compaction, 1,024 groups | Initial | 0.394 / 0.994 / 0.545 | 0.505 | 0.994 |
| Compaction, 1,024 groups | Confirmation | 1.120 / 1.200 / 0.951 / 1.112 | 1.035 | 1.136 |

Compaction component medians are retained separately. Their sums need not equal
the median total because each median is computed independently. Generate includes
historical JSON construction, model execution and final context projection; it
does not isolate model-driver overhead.

| Groups | Dataset | Prepare: baseline → candidate, ms | Generate: baseline → candidate, ms |
| ---: | --- | ---: | ---: |
| 256 | Initial | 0.275 → 0.299 | 6.217 → 6.206 |
| 256 | Confirmation | 0.256 → 0.264 | 4.516 → 4.872 |
| 1,024 | Initial | 1.205 → 1.023 | 36.395 → 18.189 |
| 1,024 | Confirmation | 0.988 → 1.057 | 18.027 → 19.019 |

Read-only source inspection found that `plan.rs` and `source.rs` differ from the
baseline only in the Runtime import name. The history-sized path through
`group_data`, `utility_record_bytes`, `utility_request`, `project` and token
estimation is unchanged. The original pinned Runtime and the candidate have
byte-identical value, model-message, request and replay implementations;
`ModelRequest::clone` shares two Arcs. The actual probe builds also have matching
Serde/JSON/Tokio/Futures versions and enabled features. The added driver performs
one call in this fixture and handles a fixed 47-byte answer. Its descriptor
checks, cancellation guards, panic containment, response assembly and cooperative
yield do not identify a new history-sized pass. This inspection only excludes
such a change within the inspected paths; it does not prove there is no
performance problem or explain the measured difference. No speculative source
change, validation shortcut or further rerun was made to improve these numbers.
One concrete lifetime difference remains: the shared driver retains its original
request through stream completion, whereas the old utility could release it
when this fixture's `start` returned. That changes when the same payload is freed,
not how many payload copies exist. Its allocator/cache contribution was not
measured and is not a causal explanation for the observed millisecond difference.

[comparison.json](comparison.json) contains both datasets, order-stratified and
combined aggregates, component medians and hashes of every evidence file. The
combined seven rounds are not order-balanced (four AB, three BA); their pooled
values must not override the balanced confirmation signal. All measurement
metadata, samples and summaries are in [initial method](paired-1009c71/method.json),
[initial samples](paired-1009c71/samples.jsonl),
[initial summary](paired-1009c71/summary.json),
[confirmation method](confirmatory-1009c71/method.json),
[confirmation samples](confirmatory-1009c71/samples.jsonl) and
[confirmation summary](confirmatory-1009c71/summary.json).

| Case | Workload | Timed boundary and checks |
| --- | --- | --- |
| Same-Turn steering | 16 ordinary model requests in one memory Session Turn | Start immediately before each fixture HTTP/SSE response write; stop when the next HTTP request body arrives. Verify the complete ordered user/assistant dialogue and each actual input's provenance, input uniqueness, original Turn binding and consecutive response indices. |
| Large history and steering | First complete 64 Turns with 4,096-byte input bodies, then make 8 requests in one new Turn | Use the same request-transition boundary. History setup is excluded and reported separately. The real Host prepares context and the real provider adapter serializes each request. |
| Compaction | Reuse Task104's deterministic Model probe with 256 and 1,024 historical groups, 1,024-byte bodies and three inner repetitions | Retain separate prepare/generate/total durations, coverage, token counts and actual utility-call counts. The model fixture is local and deterministic; this is not a provider or Session/storage latency measurement. |

`measure.py` reuses `scripts/task090_perf.py` for the real Host stdio client,
configuration and resource sampling. Only the loopback model responses are
scripted. Deliberate fixture gate waiting is excluded, but response writing,
loopback HTTP, owner scheduling and request encoding remain in the measurements.
No TUI, live-provider, saturation or capacity claim follows from these checks.
Each paired Host case must have identical request byte counts and normalized
dialogue hashes at every step. Random message IDs are checked against actual
receipts before being omitted from that content hash. Each paired compaction
repetition must match group/body sizes, source bytes, coverage and before/after
token counts, so changing or losing workload cannot be counted as a speedup.

`prepare_compaction.py` archives the exact requested commit into an external
scratch directory, copies the existing Task104 probe into that archive, and
builds with `--release --locked --offline`. The candidate substitutes only the
Runtime import name. Both normalized source hashes must match. It does not edit
the live source tree or relax dependency pins.

Both release Host executables and compaction probes were built before measurement.
Use a quiet window for the whole paired run; measurements taken during compilation
are not final results. A quiet project window does not establish machine isolation.

The output contains `method.json` with exact source/tree, binary and lockfile
hashes, `samples.jsonl` with all durations and aggregate facts, and `summary.json`
with pooled and per-round medians and ratios. Payloads, transcripts, private logs
and opaque compressed evidence are not written. Memory/CPU counters are actual
Host process observations; the configured periodic sampling is not a precise
peak-allocation measurement. No samples or warmup repetitions are silently
removed.

Correctness smoke checks completed against the original release Host for both
full Host workloads: 16 same-Turn requests with an empty history and 8 after 64
prior 4 KiB Turns. They verified input uniqueness and public response/Turn
identity; the large-history request body was 276,905 bytes. `smoke.json` records
those assertions without timings because project builds were concurrent.
The archived baseline compaction probe also passed both planned sizes, with
one utility call, a compactable prefix and fewer projected tokens in each case;
its smoke record likewise omits timings.
The measured candidate passed the same complete dialogue, provenance, Turn and
response-index assertions. Every paired Host request matched its counterpart's
byte count and normalized dialogue hash; every paired compaction repetition
matched source size, coverage, before/after tokens and one actual utility call.

The complete measurement invocations are below, from the repository root. The
baseline and candidate paths name the fixed source worktrees and previously
built artifacts recorded in each `method.json`; building is outside the timing
window. The runner refuses to overwrite an output directory, so reproduction
must use fresh output paths. These commands record the initial and confirmation
groups separately and do not select or discard any samples.

The recorded local preparation commit is not a separate remote branch. In a fresh
clone, build the candidate worktree and compaction probe from the published
`982593255fcf9b31ef1cd30a1b668420120e2ac6` instead, and use that same revision for
`--candidate-revision`. The source tree is identical, but the new method record
will correctly contain the published commit and its new archive/build identities.
Adjust workspace/build paths and use new output directories for that reproduction.

```bash
task106_args=(
  --baseline-root /workspace/scratch/205d9cb12bb1/minicore-host-baseline
  --baseline-revision 685806af4761eaaa75bae11cd70e5d7a7a0177b0
  --baseline-binary /workspace/scratch/205d9cb12bb1/build/baseline/release/minicore-host
  --baseline-compaction-build /workspace/scratch/205d9cb12bb1/build/task106-compaction-baseline/build.json
  --candidate-root /workspace/scratch/205d9cb12bb1/minicore-host-performance-1009c71
  --candidate-revision 1009c719ced338ebb05c5bacccc8c9403f882ec2
  --candidate-binary /workspace/scratch/205d9cb12bb1/build/candidate/x86_64-unknown-linux-gnu/release/minicore-host
  --candidate-compaction-build /workspace/scratch/205d9cb12bb1/build/task106-compaction-candidate-1009c71/build.json
)
PYTHONDONTWRITEBYTECODE=1 python3 tasks/106-host-runtime/performance/measure.py \
  "${task106_args[@]}" --rounds 3 \
  --out tasks/106-host-runtime/performance/paired-1009c71
PYTHONDONTWRITEBYTECODE=1 python3 tasks/106-host-runtime/performance/measure.py \
  "${task106_args[@]}" --rounds 4 \
  --out tasks/106-host-runtime/performance/confirmatory-1009c71
```
