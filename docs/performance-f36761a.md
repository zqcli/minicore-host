# Fresh local performance evidence at f36761a

## Scope and provenance

This is a fresh, bounded local checkpoint measurement, not a capacity limit,
regression claim against unavailable old data, final benchmark, live-provider
trial, or TUI acceptance. Previous raw performance files were lost when the
environment was replaced. None of their summary numbers is used below.

| Item | Identity |
| --- | --- |
| Production Host | `f36761a164c6f3ae092c5e0bbf7ab08bbcd58891` |
| Runtime | `69faf9196991524b5549499d6208a1eaf567bf56`, v0.10.0, locked Git dependency |
| Task090 harness source | `aa64dc732b52dd63f2813cb67103311973a67714` |
| Release binary SHA-256 | `ffacc545147bd4f4dae8524eadb16eaf008d755bd0ac25c3757d7f8c0bfe8c31` |
| Cargo.lock SHA-256 | `1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125` |
| SPEC v4.93 SHA-256 | `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` |

Requested worker configuration was `gpt-6-astra` / `max`; effective execution
model/settings are not independently verifiable. No production code, dependency,
shared original performance scripts, SPEC, or TUI code was changed for this task.

## Environment and execution

The successful release build used Rust/Cargo 1.90.0, two build jobs, no custom
release profile or Rust flags, and a worktree-exclusive target. It finished in
2m59.348s wall time before any measurements. The machine reported Linux 6.18.57
x86_64/glibc 2.41, AMD EPYC 9V74 80-Core Processor, **9 visible logical CPUs / 9
affinity CPUs**, 10,206,472 KiB MemTotal (about 9.73 GiB), and no swap. These are
the environment's reported values, not a guarantee of dedicated physical cores.
Python was 3.12.14. `/proc` CPU accounting used 100 ticks/s (10 ms quantum),
4,096-byte pages and CLOCK_MONOTONIC with reported 1 ns resolution. Cgroup quota
files were unavailable; no quota or whole-machine-isolation claim is made.

The coordinator confirmed other project workers were between heavy commands.
During the measured window they did not start Cargo/heavy tests, and the parent
did static work only. This is project-level coordination, not a proof that the
entire machine was idle; the visible `ps` snapshot is PID-namespace limited.
The one smoke repetition plus three formal repetitions ran from
**2026-11-10 02:55:42 to 02:55:56 UTC**, all exit 0. The formal run's environment
was captured at 02:55:45 UTC. Across its three repetitions, reported 1-minute
load average ranged 0.0024–0.0825; 5/15-minute figures retain earlier activity.
No Task090 or coordinated project build overlapped this window. No CPU pinning
or filesystem-cache flush was applied; fresh processes are not a machine-cold
cache claim. Smoke is retained separately and excluded from all following
statistics.

## Method and workloads

The actual optimized `minicore-host` process receives JSON-RPC over stdio. It
uses production Host/Agent/SessionController owners, real pinned Runtime, the
production HTTP provider and real loopback TCP/SSE. Only model replies are
scripted by a local Python server. The model name/key are explicit dummy
fixture values; no external model account or service is contacted. Logging is
the actual process-installed INFO JSONL pipeline to stderr.

Each repetition starts a fresh Host, data directory, workspace and provider
fixture, with no model warmup. The same release executable is used throughout.
Workload order reverses on even repetitions to expose, not erase, order effects.

| Workload per fresh Host | Actual condition checked |
| --- | --- |
| Empty Host | 0 Agents, 0 Sessions, no model requests |
| Persistent stock | 4 Agents / 16 Sessions; unloaded then loaded |
| Additional memory stock | 4 memory Agents / 16 memory Sessions; unloaded then loaded, alongside persistent stock |
| Paused queue | 4 memory Sessions × 8 queued 256-byte follow-up messages; 32 retained inputs and no provider request; cleared before streaming |
| Streaming | 1 Turn × 1 observer, 4 × 0, 4 × 1, 4 × 2; all intended requests reach the provider before release |
| Stream body | Eight 256-byte ASCII chunks per Turn, 2,048 bytes total, 10 ms fixture sleep between chunks |
| Cancellation | Persistent Session starts real bash leader and descendant, both blocked on FIFO; cancel after observed tool output and a deliberate 50 ms live-tool hold |
| Shutdown | Graceful control acceptance, actual owner stopped record, process supervisor stopped record and successful process exit |

These are small fixture counts, not application quotas or evidence of maximum
supported Sessions, concurrency, subscribers or output size. Observers share
one stdio connection with one promptly draining Python reader; they are not
independent slow clients. Stored/loaded contexts are almost empty, not a long
conversation corpus.

### What each timing includes

- **Message/control acceptance:** client monotonic timestamp immediately before
  pipe write/flush to complete response-line arrival. Request JSON serialization
  and client response JSON parsing are excluded. Scheduling, transport and
  actual owner acceptance are included; it is not a server-only microbenchmark.
- **Body observation:** fixture timestamp before each SSE write to complete
  matching output-notification line arrival. It includes loopback TCP, HTTP/SSE
  parsing, Runtime/Host publication, stdio serialization and client scheduling.
  It is not rendering latency. The exact contiguous body bytes and real source
  identities are checked for every observer; zero-observer bodies are checked
  through actual `turn.wait` results, never treated as observed UI output.
- **Model wait:** after every intended request reaches the fixture, the harness
  deliberately waits 50 ms. This gate has its own Host CPU/wall resource sample.
  Each request's full gate residence also includes earlier sequential setup and
  inventory queries. The first-body-from-send metric includes that wait and is
  not framework overhead. Between SSE chunks there is further deliberate pacing.
- **Cancellation completion:** separate control response, matching
  `turn.finished` observation, actual `turn.wait` terminal result, and verification
  that both known tool PIDs and their process group no longer exist. Persistence
  must be `persisted`, outcome `cancelled`, and cleanup/persistence faults absent.
  The PID check is an upper bound after owner completion, not exact death time.
  The 50 ms model gate and 50 ms tool/FIFO hold are outside cancel timing.
- **Shutdown completion:** current production has two `host.stopped` producers.
  The task-specific parser requires exactly one `host.owner` and one
  `host.process`, validates identity/schema and the owner's shutdown operation,
  and keeps their observation bounds separate. It does not assume producer
  ordering. The old event-name-only single-record assertion is not reused.
- **CPU/RSS:** Linux `/proc/<host-pid>` counters, approximately 20 ms periodic
  samples plus explicit workload boundaries. CPU is user+system time of the
  Host process and its threads only, not Python fixture/client or tool descendants.
  CPU is quantized to actual clock ticks; a zero delta does not prove zero work.
  Process CPU spans a sampled window, not unsampled startup/exit. RSS peak is
  the union of periodic and explicit observations; VmHWM is reported separately.
  RSS includes allocator retention, shared/library pages and retained state;
  sequential deltas are not precise per-Session allocations.

## Fresh results

All three formal repetitions completed the same 8-Agent/32-Session inventory,
32 paused inputs, 13 streaming Turns and one cancelled tool Turn. Each issued
exactly **14 actual HTTP model requests**, 104 emitted content chunks and 104
observer/chunk observations. Exact body, source/load/Turn identity, terminal
outcome, persistence and actual child cleanup assertions passed. All RPCs in
the traces have matched successful responses. Inventory polling produced
462/473/473 RPCs across the three repetitions, so whole-run CPU is not a fixed
operation-count microbenchmark.

### Acceptance and body observation

All values are milliseconds. Percentiles use nearest rank. Samples within a
Turn and multiple observers are correlated; with 3 or 12 acceptance samples,
the p95 is simply the maximum and is not a reliable tail estimate.

| Message acceptance | n | Min | Median | p95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Paused queue | 96 | 0.071 | 0.155 | 0.339 | 0.570 |
| 1 Turn, 1 observer | 3 | 0.208 | 0.251 | 0.294 | 0.294 |
| 4 Turns, 0 observers | 12 | 0.160 | 0.346 | 1.089 | 1.089 |
| 4 Turns, 1 observer each | 12 | 0.147 | 0.246 | 0.799 | 0.799 |
| 4 Turns, 2 observers each | 12 | 0.164 | 0.196 | 0.433 | 0.433 |

| Body notification after fixture write begins | n | Min | Median | p95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 Turn, 1 observer | 24 | 0.199 | 0.254 | 0.536 | 2.173 |
| 4 Turns, 0 observers | 0 | not observed | not observed | not observed | not observed |
| 4 Turns, 1 observer each | 96 | 0.076 | 0.241 | 1.168 | 2.870 |
| 4 Turns, 2 observers each | 192 | 0.109 | 0.302 | 1.109 | 1.925 |

First observed body from message send was **52.988–59.602 ms**, including the
deliberate model gate. Per-request fixture residence before the first write
was **51.473–58.266 ms**. These totals must not be attributed to the Host.
The small acceptance differences are noisy/order-dependent observations, not
evidence that two observers are faster than one or that fanout is free.

### Cancellation and owner completion

Values below are elapsed milliseconds from the cancel request's actual pipe
write. Each row is kept distinct: a quick acceptance is not cleanup completion.

| Witness | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Control accepted | 0.486 | 0.464 | 0.544 |
| Actual `turn.wait` terminal result returned | 2.966 | 3.069 | 5.212 |
| Matching `turn.finished` observed | 3.030 | 3.164 | 5.313 |
| Both PIDs and process group verified absent | 3.243 | 3.438 | 5.711 |

The real tool result was cancelled, the terminal result was persisted without
cleanup/persistence faults, and its retained tool history matched `toolcall.get`.
These are actual owner-completion/cleanup witnesses, not receipt-only success.
They do not locate the exact internal join or process-death instruction.
Before cancellation, the model gate was 50.158–51.101 ms, tool-ready observation
followed release by 4.428–6.144 ms, and tool-ready to cancel was 50.722–50.915 ms.
The deliberate FIFO hold accounted for 50.227–50.275 ms and 0 measured Host CPU
ticks in all three runs; tool-descendant CPU is not measured.

For graceful Host shutdown, elapsed milliseconds from that control write were:

| Witness | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Control accepted | 0.553 | 0.567 | 0.660 |
| `host.owner` stopped record observed | 3.125 | 2.058 | 2.280 |
| `host.process` stopped record observed | 3.252 | 2.114 | 2.339 |
| Successful process exit observed | 8.053 | 3.893 | 3.973 |

### CPU and memory

RSS values are KiB at the end of each approximately 400 ms quiet stock phase.
They are sequential snapshots in the same process, not standalone per-Session
allocation tests. Every stock/paused holding phase had zero measured CPU-tick
delta; entity creation/loading/query CPU is outside those holding windows.

| Stock/queue condition | Run 1 RSS | Run 2 RSS | Run 3 RSS |
| --- | ---: | ---: | ---: |
| Empty Host | 13,656 | 13,300 | 13,140 |
| 16 persistent Sessions unloaded | 14,544 | 14,528 | 14,412 |
| 16 persistent Sessions loaded | 15,164 | 15,060 | 14,900 |
| Plus 16 memory Sessions unloaded | 15,388 | 15,296 | 15,176 |
| All 32 Sessions loaded | 15,432 | 15,340 | 15,364 |
| Plus 32 paused inputs | 15,612 | 15,612 | 15,436 |

Streaming resource windows include subscription/setup, sends, inventory,
the model gate, paced response, result queries and unsubscribe. Wall windows
were 126.833–137.207 ms. The explicit 50 ms model-gate windows lasted
50.221–50.674 ms; all 12 had zero measured Host CPU-tick delta.

| Streaming window Host CPU, ms | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| 1 Turn, 1 observer | 10 | 0 | 10 |
| 4 Turns, 0 observers | 10 | 10 | 0 |
| 4 Turns, 1 observer each | 20 | 10 | 20 |
| 4 Turns, 2 observers each | 10 | 20 | 20 |

| Full sampled process window | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Wall, ms | 3,242.051 | 3,197.586 | 3,188.120 |
| Host CPU, ms | 150 | 120 | 130 |
| CPU as % of one core over that window | 4.627 | 3.753 | 4.078 |
| Peak sampled RSS, KiB | 17,060 | 18,196 | 18,132 |
| Maximum observed VmHWM, KiB | 17,060 | 18,196 | 18,132 |
| RSS samples present / missing | 190 / 1 | 189 / 0 | 189 / 0 |

The maximum sampled RSS was **16.66–17.77 MiB**. Run 1's final periodic sample
saw a zombie process with no VmRSS/VmHWM; that missing value is counted, not
invented as zero. Other explicit samples still participate in the peak union.
These are sampled windows and observed high-water counters, not exact complete
lifetime CPU or a separately captured final process RSS maximum.

The result shows these bounded real-owner paths functioning responsively on
this checkpoint. It does not establish a capacity ceiling, speedup, regression
ratio or latency guarantee. There is no comparable recovered raw baseline.

## Reproduction and evidence

Build and measurement use the isolated Task090 worktree/target. Build resource
use is not included in Host performance samples. The first wrapper could not
find `/usr/bin/time` and never invoked Cargo; its output is retained separately.
Bash `time` then wrapped the successful locked/offline release build.

```sh
export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-task090-target
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
cargo build --release -p minicore-host --locked --offline
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_perf_test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_perf.py \
  --binary /workspace/shared/minicore-host-task090-target/release/minicore-host \
  --source-revision f36761a164c6f3ae092c5e0bbf7ab08bbcd58891 \
  --out tasks/090-fresh-performance/evidence/smoke-aa64dc7 --runs 1
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_perf.py \
  --binary /workspace/shared/minicore-host-task090-target/release/minicore-host \
  --source-revision f36761a164c6f3ae092c5e0bbf7ab08bbcd58891 \
  --out tasks/090-fresh-performance/evidence/measured-aa64dc7 --runs 3
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_summarize.py \
  tasks/090-fresh-performance/evidence/measured-aa64dc7/results.json
PYTHONDONTWRITEBYTECODE=1 python3 tasks/090-fresh-performance/verify_evidence.py
```

These are the actual commands; reruns must choose new output directories rather
than overwrite either retained set. The complete shell transcript records the
window's start/end and exit-success progression.

Source, binary, raw events and derived result hashes are recorded under
[`tasks/090-fresh-performance/evidence`](../tasks/090-fresh-performance/evidence).
The existing source guard compares production files/manifest/lock against the
fixed SHA. Each output directory must be new. Raw logs retain every JSON-RPC
request/response, observation, actual process sample, provider request/emission,
diagnostic and cleanup witness. Failed runs must remain failed evidence, not
be folded into successful descriptive summaries.

Key SHA-256 values for the formal set:

| File under `evidence/measured-aa64dc7/` | SHA-256 |
| --- | --- |
| `results.json` | `33e3eb97b4698861677bde8994e31e3564a71f3a402737dc9f26684174c90c33` |
| `run-01.jsonl` | `6c6044d1e61978e159674ee02280e91cafa31693babea293f77ab393cc5d9032` |
| `run-02.jsonl` | `6b7a3b010d3dfd366e4e68876054b9ea870ab41607e747ab00f42ea8bc7fc593` |
| `run-03.jsonl` | `c34adccb498806c01aa1c2ac342f70d350c05e3a9da45f623c73d97eab4ae6b2` |
| `summary.json` | `3dc7a4f8c898e323b12269e325002f0423d6d67afeaf21623f4abd29ca9accae` |

New parser fixtures (8), existing public-query fixtures (15), existing RSS
fixtures (5), Rust formatting, repository whitespace and production-source
identity checks passed. A raw-log audit matched successful RPC responses to
all sends, recomputed the 45 queue/stream acceptance timings and 104 body
observation timings per run, checked 14 model requests/104 emissions, actual
cleanup witnesses and both stopped producers, and revalidated the RSS union
and summaries. The smoke has its own complete trace and summary, not additional
formal repetitions. Rust all-target test/Clippy/doctest/doc gates were not rerun
by this evidence-only task; the coordinator owns broader integration validation.

## Unmeasured and interpretation limits

No live model/provider, WAN/TLS, WSS fanout, browser/TUI rendering, real user
workflow, long-running soak, large transcript/body, compaction, high-load
saturation, slow/disconnected consumer, cold persistent history query, fairness,
OOM or throughput maximum is measured. Host CPU does not isolate network/parser
cost from Runtime/owner cost. Fixture/tool waits are named separately, not
subtracted to manufacture pure framework time. No product latency/SLO threshold
is assigned. Small correlated samples cannot establish statistical significance.

Production diagnostics continue to be developed after this fixed checkpoint.
This report does not validate later commits, and it does not expand the separate
saved-Turn-result repair scope. The original performance reports/raw traces are
unavailable, so no historical before/after regression ratio is claimed.
