# Task 106 focused performance checks

The measurements are pending the final candidate commit/release build and a
coordinated window without project compilation. Script smoke checks use the
fixed Host baseline only; their durations are not performance evidence.

The baseline is Host `685806af4761eaaa75bae11cd70e5d7a7a0177b0` and its pinned
Runtime `3aa47629929906832dcb309afb7d242c1ae3e53f`. The candidate will use the exact
reviewed Host source commit. Both variants run the same workloads and assertions.

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

After both release Host executables and compaction probes are available, run
three paired rounds. The order alternates baseline/candidate and
candidate/baseline. Use a quiet window for the whole paired run; do not treat
measurements taken during compilation as final results. A quiet project window
does not establish machine isolation.

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
Candidate checks and paired measurements remain pending.
