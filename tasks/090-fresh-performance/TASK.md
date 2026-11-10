# Task 090: fresh fixed-checkpoint performance evidence

## Assignment and boundary

- Fixed production source: `f36761a164c6f3ae092c5e0bbf7ab08bbcd58891`.
- Branch/worktree: `task/090-fresh-performance`,
  `/workspace/shared/minicore-host-task090`.
- Worktree-exclusive target: `/workspace/shared/minicore-host-task090-target`.
- Requested worker configuration: `gpt-6-astra` / `max`, as relayed by the
  coordinator. Effective runtime model/settings cannot be independently verified.
- Own only this task, its new evidence, `docs/performance-f36761a.md`, and
  task-specific new Python scripts. No production, SPEC, shared documentation,
  original performance script, lockfile, or manifest edits. No push.

Read `AGENTS.md`, SPEC performance section (2302–2310), Task074 and the recovered
Task058 harness/summarizer. The old performance documents and raw measurements
are absent from this restored checkout. Historical numbers are not reused as
fresh evidence. This work measures a fixed checkpoint with controlled local
HTTP responses, not a final benchmark, live-provider trial, or TUI validation.

## Harness adaptation

The original harness SHA-256 at this checkpoint is
`f332e227d6c6094d5f2856c6ee75d3685e6937a782490f00ade844c089080373`.
The old single `host.stopped` assertion no longer matches current production:
`host.owner` records its actual stopped barrier, and `host.process` records
supervisor/transport completion. The new task-specific copy requires both
records exactly once, validates Host identity/schema and the owner's actual
shutdown operation, and keeps their receive timestamps separate. It does not
assume cross-producer diagnostic ordering. Eight pure parser fixtures cover
this change. No fallback to an old log shape is added.

The copy preserves the existing public DTO/body/cleanup assertions, actual
Host/Runtime/HTTP path, workload, fresh-output guard and fixed-source guard.
It adds a measured resource boundary around the deliberate 50 ms model gate;
the old 10 ms/chunk SSE pacing and 50 ms live FIFO tool hold stay explicit.
`task090_summarize.py` calls the unchanged descriptive summarizer and adds these
separate gate/owner measurements. Neither summarizer applies passing thresholds.

## Build and measurement execution

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-task090-target
cargo build --release -p minicore-host --locked --offline
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_perf_test.py
```

The first build wrapper attempted `/usr/bin/time`, which is unavailable; Cargo
was not run by that attempt. Its output is retained separately. The retry uses
Bash's `time` builtin. Build and measurement resource usage are kept separate.
The successful release compile finished before the coordinated no-build interval.
The execution sandbox exposes only its PID namespace to `ps`; an empty process
list is not evidence that other contributors or the machine are idle.

Measured fresh workloads are the existing bounded 8 Agents/32 Sessions stock,
32 paused inputs across 4 Sessions, 1 or 4 active model requests with 0/1/2
observers per Session, and a real bash process group cancelled under its
persistent Session owner. These counts describe fixtures, not product limits.
The smoke repetition and all three formal fresh-Host repetitions exited 0.
Their raw logs are retained separately; no smoke sample is added to the formal
statistics. There was no failed Host run. The only failed command was the
pre-build unavailable `/usr/bin/time` wrapper noted above.

## Completed measurement and evidence

- Committed harness: `aa64dc732b52dd63f2813cb67103311973a67714`,
  `test(perf): qualify fixed-checkpoint completion measurements`.
- Release binary SHA-256:
  `ffacc545147bd4f4dae8524eadb16eaf008d755bd0ac25c3757d7f8c0bfe8c31`.
- Runtime v0.10.0: `69faf9196991524b5549499d6208a1eaf567bf56`, unchanged lock.
- Build: Rust/Cargo 1.90.0; release, 2 jobs; 2m59.348s wall; exit 0.
- Actual window: **2026-11-10T02:55:42Z–02:55:56Z**, confirmed by shell timestamps.
  The coordinator confirmed workers 053/088/089/091 were not running Cargo or
  heavy tests, 087 had exited, and the parent did static work only. The window
  was released immediately after the four runs. This is project coordination,
  not verified whole-machine isolation.
- Actual environment: Linux 6.18.57/x86_64/glibc 2.41, reported EPYC 9V74,
  9 logical/affinity CPUs, about 9.73 GiB MemTotal, no swap, Python 3.12.14,
  100 Hz process CPU counters. No CPU pinning/cache flushing; cgroup quotas
  could not be read.

The complete report is [performance-f36761a.md](../../docs/performance-f36761a.md).
It records actual inventory, queue, parallel and observer conditions, precise
timing definitions, artificial waits, CPU/RSS results and unmeasured scope.
Selected three-run results:

- Queued message acceptance: n=96, median 0.155 ms, p95 0.339 ms.
- Streaming body observation median 0.241–0.302 ms by observed workload;
  per-workload p95 0.536–1.168 ms. This is fixture-write to complete stdio
  notification line, not TUI display or pure framework time.
- Cancel acceptance 0.464–0.544 ms; actual `turn.wait` terminal result
  2.966–5.212 ms; both real bash PIDs/process group verified absent by
  3.243–5.711 ms. All cancelled results persisted without cleanup faults.
- Host CPU 120–150 ms over sampled windows of 3.188–3.242 s, including deliberate
  idle/model/tool waits. Peak sampled RSS 16.66–17.77 MiB. One end-of-run zombie
  sample lacked RSS; it remains missing, not zero.
- 14 actual HTTP requests, 13 completed streaming Turns plus one cancelled
  tool Turn, and 104 emitted content chunks per formal run. These counts are
  workload descriptions, never capacity limits or fabricated passing targets.

The same controlled 50 ms model gate, 10 ms/chunk pacing and 50 ms live FIFO
hold are explicit. No live model, WAN, TLS/WSS, slow observer, TUI, compaction,
large corpus, long soak, saturation or saved-turn cold-read test ran. The
separate saved-turn-result repair is not modified or certified by this task.
This checkpoint predates later diagnostics and is not their final benchmark.

## Commands, consistency checks and integrity

Exact measurement commands, coordinator-window scope and timestamps are in
[measurement-command.txt](evidence/measurement-command.txt); complete build
output is in [release-build.txt](evidence/release-build.txt). The source/binary
checks, build configuration and original-to-new harness diff are retained too.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_perf_test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task059_projection_test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task058_rss_test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_summarize.py \
  tasks/090-fresh-performance/evidence/measured-aa64dc7/results.json
PYTHONDONTWRITEBYTECODE=1 python3 tasks/090-fresh-performance/verify_evidence.py
cargo fmt --all -- --check
./scripts/check-whitespace.sh
git diff --check
git diff --exit-code f36761a164c6f3ae092c5e0bbf7ab08bbcd58891 -- \
  crates Cargo.toml Cargo.lock SPEC.md scripts/task058_perf.py \
  scripts/task058_summarize.py scripts/task058_rss_test.py scripts/task059_projection_test.py
sha256sum -c tasks/090-fresh-performance/evidence/SHA256SUMS
```

The 8 new pure parser cases plus 15 unchanged projection and 5 RSS cases pass.
Fresh release execution and original-source comparison pass. The read-only
raw audit matches every RPC without errors, reconstructs 45 message acceptance
and 104 body observation timings per run, verifies actual requests/emissions,
cleanup/stopped witnesses, sampled resource aggregation and deterministic
summaries. These pure/consistency checks are not additional measured Host runs.
Strict Rust Clippy, all-target tests, doctests and rustdoc were **not rerun** by
this evidence-only task; broader integration gates and independent review remain
the coordinator's responsibility. No full-gate or final product-completion claim
is made here.

Formal `results.json` SHA-256:
`33e3eb97b4698861677bde8994e31e3564a71f3a402737dc9f26684174c90c33`.
All three raw JSONL hashes, the smoke set, summaries and command outputs are in
the task-local checksum manifest. The report reproduces the principal formal
set hashes. Raw files are committed to prevent another source-only recovery
from being mistaken for preserved performance evidence. This task performed
no push, publication, merge, or shared-document edit.
