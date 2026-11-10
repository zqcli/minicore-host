# Final composed backend: fresh local performance at c22d8c8

## Scope and fixed source

This is a new, bounded SPEC section 13 measurement of the composed backend.
No performance values are copied from the earlier f36761a report. It is not a
capacity claim, latency guarantee, live-provider trial, TUI validation or complete
first-version acceptance. The source includes the final Task053 diagnostics,
reviewed compaction Fault handoff and saved-result query composition.

| Item | Identity |
| --- | --- |
| Host source | `c22d8c8af058b756d5f37bc6d1a01f1dc7749934` |
| Host tree | `a805cb4f6bf0d817fdf5d06226f8638e14ba95e6` |
| Runtime | `3aa47629929906832dcb309afb7d242c1ae3e53f`, v0.10.0, published locked Git dependency |
| Release binary SHA-256 | `7644c8ea0aad53f0627f1677c851bc6a859f656b2571cad687e0427078dcf615` |
| Cargo.lock SHA-256 | `b8aa55600f443e7ff923096f37da833049d106078d5df44aa16362d70e11873b` |
| SPEC v4.93 SHA-256 | `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` |
| Unchanged measurement harness SHA-256 | `6c62caf417c61d5fc93807459573057847c2e80fa659a576ac3db4cc7326729f` |

The actual `minicore-host` binary was built once with official Rust/Cargo 1.90.0,
`cargo build --release -p minicore-host --locked --offline`, two Cargo jobs,
no custom release flags, and a new worktree-exclusive target. The successful build
took 163.502 s wall time, entirely before measurement. The executable is
23,010,656 bytes. Production, dependencies, SPEC, TUI and all existing measurement
scripts are unchanged. Requested worker configuration was gpt-6-astra/max; the
actual effective model/runtime setting is not independently attested.

The coordinator separately passed the c22 composition's 939 Rust cases,
4 doctests and 20 Python tests. That functional gate is not performance evidence
and was not rerun by this artifact-only task. Later documentation and the addition
of eight existing parser checks to the root gate do not change this binary's
fixed source. This run measures the public Runtime3aa / final 939-case composition,
not Task090's older Runtime69faf91 checkpoint or an earlier 937-case composition.

## Method and workloads

The existing `scripts/task090_perf.py` and its summarizers/helpers run without
semantic changes. Each repetition starts a fresh real Host process, data root,
workspace and loopback HTTP/SSE model fixture. Production Host/Agent/Session,
Runtime, provider, filesystem and bash tool code execute. Only model responses
are scripted, using explicit dummy fixture credentials. INFO JSONL diagnostics
use the real process-installed stderr pipeline; stdout remains RPC.

There is one retained smoke repetition and three formal repetitions. Smoke is
excluded from every reported statistic. The release executable is identical for
all four; there is no model warmup. Streaming workload order reverses on even
repetitions. Fresh processes do not mean cold OS or filesystem caches.

| Workload per fresh Host | Conditions |
| --- | --- |
| Empty and stored entities | Empty Host; then 4 persistent Agents / 16 Sessions, unloaded and loaded |
| Memory entities | Add 4 memory Agents / 16 Sessions, unloaded then loaded, retaining persistent stock |
| Queued inputs | 4 paused memory Sessions × 8 messages × 256 bytes; 32 inputs, no model requests; cleared before streams |
| Streaming | 1 Turn × 1 observer; 4 × 0; 4 × 1; 4 × 2, with every intended HTTP request actually gated before release |
| Stream bodies | 8 chunks × 256 ASCII bytes = 2,048 bytes per Turn; deliberate 10 ms sleeps between chunks |
| Cancellation | Persistent Session, real bash leader plus descendant blocked on FIFO, actual output witnessed before cancel |
| Shutdown | Graceful receipt, separate actual owner/process stopped diagnostics, successful process exit |

Message/control latency is client monotonic time immediately before pipe
write/flush until the complete response line arrives. Request serialization and
response parsing are excluded, but IPC, scheduling and real owner acceptance
are included. Body observation runs from fixture pre-SSE-write timestamp to the
matching complete output-notification line. It includes loopback/provider parsing,
Runtime/Host publication, stdio and client scheduling, not rendering. Exact bodies,
contiguous byte ranges and actual source identities are asserted. Zero-observer
Turn bodies are verified through `turn.wait`, not assigned invented display times.

Every streaming batch deliberately holds all arrived model requests for 50 ms.
Its CPU/wall interval is sampled separately. Each request's full fixture residence
also includes earlier setup and inventory; first body from message send includes
these waits. The real tool has a separate 50 ms model gate and 50 ms FIFO hold
before cancellation. None is subtracted to manufacture a pure framework overhead.

Cancellation reports receipt, matching `turn.finished`, actual terminal
`turn.wait`, and verification that both known PIDs and their process group are
absent. Terminal persistence must be `persisted`, outcome `cancelled`, with no
cleanup/persistence faults. The PID witness is an upper bound after owner
completion, not an exact death timestamp. Shutdown likewise separates receipt,
`host.owner`, `host.process`, and exit; no producer ordering is assumed.

Host-only CPU uses real `/proc` user+system tick counters, excluding the Python
fixture/client and bash descendants. A zero tick delta means no tick observed,
not zero work. RSS uses approximately 20 ms periodic samples plus explicit
workload boundaries; peak sampled RSS and VmHWM are separate. Missing fields are
counted, never replaced with zero. Stock snapshots are sequential allocator/state
observations, not precise per-Session allocations or whole-lifetime CPU totals.

## Environment and measured results

### Environment and execution window

The machine reported Linux 6.18.57 x86_64, glibc 2.41, AMD EPYC 9V74,
9 logical / 9 affinity CPUs, 10,206,472 KiB MemTotal (about 9.73 GiB), no swap,
and Python 3.12.14. These are reported platform facts, not dedicated physical-core
guarantees. CPU counters have 100 ticks/s (10 ms quantum), pages are 4,096 bytes,
and CLOCK_MONOTONIC reports 1 ns resolution. Cgroup quota files were unavailable;
no quota value is inferred.

The actual smoke-plus-formal window was **2026-11-10 03:53:51–03:54:06 UTC**.
The formal environment capture was at 03:53:55 UTC. All four fresh Hosts exited
successfully through real graceful shutdown. The coordinator waited for root,
review and CLI builds to finish and paused publisher local fetch/large-file work;
no project heavy build was started in the window. This is not whole-machine
isolation. The process snapshot is PID-namespace limited. Formal 1-minute load
ranged 0.1851–0.2188; 5/15-minute averages retain earlier activity. There was no
CPU pinning or OS cache flush.

### Actual sample inventory

Each formal repetition completed 8 Agents / 32 Sessions, 32 paused inputs,
13 completed streaming Turns and one cancelled real-tool Turn. Each made exactly
14 real HTTP model requests, emitted 104 body chunks and retained 104
observer/chunk pairs (fanout observes one chunk twice; the zero-observer workload
has no body observations). The live exact-body, source/Load/Turn, persistence and
real child-cleanup assertions passed. The three raw traces contain 396 / 396 / 440
matched successful RPCs; inventory polling means whole-run CPU is not a fixed
operation-count microbenchmark. No failed or slow sample was discarded. Smoke
remains separate from the following three-run statistics.

### Message acceptance and body observation

All values are milliseconds; percentiles use the unchanged nearest-rank
summarizer. Samples within a Turn and across its observers are correlated. For
3 or 12 samples, p95 equals the maximum and is not a credible tail guarantee.

| Message acceptance | n | Min | Median | p95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Paused queue | 96 | 0.083 | 0.153 | 0.266 | 1.322 |
| 1 Turn, 1 observer | 3 | 0.219 | 0.220 | 0.257 | 0.257 |
| 4 Turns, 0 observers | 12 | 0.158 | 0.283 | 0.743 | 0.743 |
| 4 Turns, 1 observer each | 12 | 0.115 | 0.198 | 0.369 | 0.369 |
| 4 Turns, 2 observers each | 12 | 0.156 | 0.233 | 0.296 | 0.296 |

| Body notification after fixture write begins | n | Min | Median | p95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 Turn, 1 observer | 24 | 0.197 | 0.274 | 0.720 | 0.864 |
| 4 Turns, 0 observers | 0 | not observed | not observed | not observed | not observed |
| 4 Turns, 1 observer each | 96 | 0.093 | 0.223 | 0.690 | 1.183 |
| 4 Turns, 2 observers each | 192 | 0.106 | 0.320 | 1.416 | 2.148 |

First observed body from message send ranged **53.156–57.828 ms**, including the
explicit model gate and setup. Per-request fixture residence before its first
SSE write was 51.768–56.807 ms. These waits are not Host overhead. Differences
between one- and two-observer medians do not establish fanout cost or benefit.

### Control acceptance versus actual completion

Each cell below is elapsed milliseconds from that cancel request's pipe write.
Quick acceptance is not process cleanup or persistence confirmation.

| Cancellation witness | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Control accepted | 0.410 | 0.534 | 0.490 |
| Actual terminal `turn.wait` returned | 23.065 | 1.285 | 1.770 |
| Matching `turn.finished` observed | 23.410 | 1.401 | 1.646 |
| Both PIDs and group verified absent | 24.244 | 1.640 | 2.029 |

The **24.244 ms** Run 1 cleanup witness is retained, not replaced by another run.
Its receipt was at 0.410 ms, final tool output at 22.367 ms, `toolcall.finished`
at 22.667 ms and terminal `turn.wait` at 23.065 ms. Most observed delay therefore
precedes tool completion, rather than being only a slow receipt or late PID check.
The separate smoke also had a 24.616 ms cleanup witness; it remains excluded from
formal statistics.

The unchanged `host-local-tools/src/command.rs::confirmed_group_gone` can wait
20 ms before repeating its actual process-group absence probe. That is a plausible
explanation for this scale of delay, **not a proven attribution**: this run has
no syscall tracing or internal branch timestamps. The file is unchanged between
f36761a and c22. No logic change or selected rerun was made; these small, unpaired
samples do not establish a systematic regression or logging bottleneck. The exact
observation sequence is retained in
[`cancel-boundary-audit.json`](../tasks/098-composed-performance/evidence/cancel-boundary-audit.json).

All cancelled Turns were persisted with no cleanup/persistence fault; actual
ToolCall results matched retained native history. Before cancel timing began,
the tool's model gate lasted 50.158–50.171 ms, release-to-tool-output observation
3.501–3.812 ms, and the explicit live FIFO hold 50.193–50.266 ms. Each FIFO hold
had zero observed Host CPU ticks; descendant CPU is not measured.

Graceful shutdown is also split, in milliseconds from its control write:

| Shutdown witness | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Control accepted | 0.322 | 0.438 | 0.359 |
| `host.owner` stopped observed | 2.469 | 2.138 | 2.152 |
| `host.process` stopped observed | 2.667 | 2.175 | 2.221 |
| Successful process exit observed | 7.705 | 8.007 | 7.765 |

### Host CPU and memory

RSS is KiB at the end of each approximately 400 ms stock/queue hold. All these
holding intervals had zero measured CPU-tick delta; creation/load/query costs
occur outside the holds. Sequential RSS differences include allocator retention
and shared pages and cannot yield exact per-Session allocation costs.

| Stock/queue condition | Run 1 RSS | Run 2 RSS | Run 3 RSS |
| --- | ---: | ---: | ---: |
| Empty Host | 11,740 | 11,788 | 11,788 |
| 16 persistent Sessions unloaded | 13,332 | 13,440 | 13,384 |
| 16 persistent Sessions loaded | 13,700 | 13,800 | 13,704 |
| Plus 16 memory Sessions unloaded | 13,992 | 14,088 | 13,988 |
| All 32 Sessions loaded | 14,036 | 14,264 | 14,164 |
| Plus 32 paused inputs | 14,240 | 14,296 | 14,212 |

Streaming windows include subscription/setup, sends, inventory, the 50 ms gate,
paced responses, result queries and unsubscribe. They lasted 125.907–142.088 ms.
The 12 explicit model-gate windows lasted 50.208–50.537 ms; each had zero observed
Host CPU-tick delta. The CPU values below retain the 10 ms clock quantum.

| Streaming window Host CPU, ms | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| 1 Turn, 1 observer | 10 | 10 | 10 |
| 4 Turns, 0 observers | 10 | 0 | 10 |
| 4 Turns, 1 observer each | 10 | 20 | 10 |
| 4 Turns, 2 observers each | 10 | 20 | 30 |

| Whole sampled process window | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Wall, ms | 3137.861 | 3128.884 | 3138.623 |
| Host CPU, ms | 100 | 110 | 120 |
| CPU as % of one core | 3.187 | 3.516 | 3.823 |
| Peak sampled RSS, KiB | 16,288 | 16,268 | 16,288 |
| Maximum observed VmHWM, KiB | 16,288 | 16,268 | 16,288 |
| RSS samples present / missing | 186 / 0 | 186 / 0 | 186 / 0 |

Peak sampled RSS was **15.887–15.906 MiB**. No RSS/HWM observations were missing
in these formal runs; a missing value would remain unknown, not numerical zero.
These are sampled windows and observed counters, not exact lifetime CPU, private
heap size or final exit-time memory peak. The measurements describe this bounded
real-owner workload only, with the retained cancellation tail above.


## Reproduction, retained evidence and limits

Evidence retains the actual environment, exact commands, original results,
unchanged descriptive summaries and lossless gzip raw JSONL. The read-only
Task098 verifier checks both compressed and original hashes, recomputes summaries,
RPC acceptance and body-observation times from raw records, verifies cancellation
and observed shutdown boundaries, and checks real model request/cleanup counts
and the periodic/explicit RSS union. It does not run a Host
or replace the live harness's business/identity/cleanup assertions.

The 3,407,255 original JSONL bytes are preserved in four gzip files totaling
251,008 bytes, with compression timestamp zero. Original and compressed byte
counts and SHA-256 values are in
[`raw-compression.json`](../tasks/098-composed-performance/evidence/raw-compression.json).
Every decompressed file was compared byte-for-byte before its uncompressed
original was moved to a separate local backup. The public verifier reads gzip
directly and needs neither that backup nor the Host executable. Results and
summaries remain ordinary JSON. Existing Task090 evidence is unchanged.

Verify the retained evidence read-only from the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tasks/098-composed-performance/verify_evidence.py
(cd tasks/098-composed-performance/evidence && sha256sum --check SHA256SUMS)
```

For a new measurement, build the fixed source in an exclusive target and choose
new output directories, never overwrite either retained set:

```sh
cargo build --release -p minicore-host --locked --offline
python3 scripts/task090_perf.py --binary /absolute/path/to/release/minicore-host \
  --source-revision c22d8c8af058b756d5f37bc6d1a01f1dc7749934 \
  --out /new/evidence/smoke --runs 1
python3 scripts/task090_perf.py --binary /absolute/path/to/release/minicore-host \
  --source-revision c22d8c8af058b756d5f37bc6d1a01f1dc7749934 \
  --out /new/evidence/formal --runs 3
python3 scripts/task090_summarize.py /new/evidence/formal/results.json
```

The actual completed commands/environment are in
[`measurement-command.txt`](../tasks/098-composed-performance/evidence/measurement-command.txt)
and [`build-provenance.json`](../tasks/098-composed-performance/evidence/build-provenance.json).
The unchanged 8 parser, 15 query-projection and 5 RSS Python fixtures passed before
measurement. Source/harness identity, read-only raw audit, reproducible summaries,
formatting and repository whitespace checks passed afterward. This task did not
rerun the separate Rust workspace test/Clippy/doc gate and performs no push.

This is not a controlled historical A/B benchmark. The f36761a report remains
separate; its results are not reused as these samples. There is no predeclared
performance threshold or capacity limit. Small correlated sample sets, especially
3 or 12 message samples, cannot establish reliable tails or statistical
significance. All observers use one promptly draining stdio reader, not slow or
independent network clients. Stock contexts are almost empty.

Unmeasured: live model/provider, WAN/TLS/WSS fanout, browser/TUI rendering,
real-user workflow, long history/body, saved-history cold-query latency,
compaction/retry throughput, soak, saturation, fairness, OOM, slow/disconnected
consumers or maximum scale. CPU is not isolated into parser versus Runtime versus
owner costs. No production/harness changes, extra benchmark platform, quotas or
worker push are part of this task.
