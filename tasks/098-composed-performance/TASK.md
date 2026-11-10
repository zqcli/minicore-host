# Task098: Measure the final composed backend

## Fixed scope

- Production source is `c22d8c8af058b756d5f37bc6d1a01f1dc7749934`, tree
  `a805cb4f6bf0d817fdf5d06226f8638e14ba95e6`, including the final Task053 slices.
  The coordinator's 939 Rust / 4 doctest / 20 Python full gate is a separate run,
  not a performance result and not repeated or claimed by this evidence-only task.
- Runtime is the published locked Git `3aa47629929906832dcb309afb7d242c1ae3e53f`,
  version 0.10.0. This is later than Task090's f36761a / Runtime69faf91 checkpoint.
- The existing `scripts/task090_perf.py`, summarizer and query/resource helpers are
  used unchanged. There are no production, dependency, SPEC, TUI, historical
  evidence or measurement-harness edits.
- Only this task directory and `docs/performance-c22d8c8.md` are owned here. The
  worktree and Cargo target are exclusive. No worker push is authorized.
- Requested engineering configuration is gpt-6-astra/max; no effective-runtime
  attestation is made.

## Method fixed before measurement

Build the actual executable in locked offline release mode with official
Rust/Cargo 1.90.0 and two build jobs. Capture source, harness, lock and binary
hashes. After notifying the coordinator that compilation has ended, run one
smoke plus three formal fresh-Host repetitions in a coordinated short window
without new project heavy builds. Smoke is retained but excluded from formal
statistics. Do not drop failures or impose a product performance threshold.

Use the existing real process/stdIO RPC, production owners, actual Runtime,
loopback HTTP/SSE provider fixture and real bash/FIFO descendant cleanup path.
Separate actual message/control acceptance, body observation, cancellation
completion, source/observer identities, deliberate fixture/tool wait, Host-only
CPU ticks and actual sampled RSS/HWM. This is not live-provider, TUI, capacity,
whole-machine-isolation or server-only framework-overhead validation.

Keep every raw sample. Publish raw JSONL losslessly as deterministic gzip with
both original and compressed hashes and byte counts, plus ordinary results,
summary, environment and method/provenance files. The task-local read-only audit
is adapted from the existing Task090 verifier solely for this fixed source and
compressed evidence; it never launches a Host or manufactures samples.

## Actual results and validation

The locked offline release build passed in 163.502 s wall time, before the
coordinated window. Its actual binary SHA-256 is
`7644c8ea0aad53f0627f1677c851bc6a859f656b2571cad687e0427078dcf615`.
The unchanged parser/query/RSS checks passed 8 + 15 + 5 cases before measurement.
The coordinator confirmed root/review/CLI builds had ended and paused publisher
local fetch/large-file work. This is project coordination, not machine isolation.

One smoke and three formal fresh-Host runs all exited 0 between
2026-11-10 03:53:51 and 03:54:06 UTC. Every run made 14 actual HTTP model requests,
emitted 104 chunks and checked 104 observer/chunk timing pairs, with successful
exact-body/identity/persistence/real-process-cleanup assertions. Smoke is retained
separately. The formal runs have 396 / 396 / 440 successful matched RPCs; no
failure or slow sample was removed. No additional load run was performed.

Formal message-accept medians were 0.153–0.283 ms across the measured workloads;
body-observation medians were 0.223–0.320 ms. Whole sampled Host CPU was
100 / 110 / 120 ms and peak sampled RSS 16,288 / 16,268 / 16,288 KiB.
Cancel receipt was 0.410 / 0.534 / 0.490 ms; actual PID/group absence witnesses
were 24.244 / 1.640 / 2.029 ms. The 24.244 ms sample and the similar smoke sample
are preserved. Its raw timeline places most delay before tool completion;
the existing 20 ms group-absence polling is only a candidate explanation,
not a claimed branch/syscall observation or proved regression. No production
change, selected rerun, strict historical A/B comparison or threshold is added.

The read-only task-local audit passed on all four compressed raw files. It
checks exact raw/compressed hashes, successful RPC matching, original message
timing, observed body timing, cancellation and observed shutdown boundaries,
actual model/cleanup counts, RSS sample union and regenerated summaries.
The 3,407,255 raw JSONL bytes compress losslessly to 251,008 bytes. The ordinary
results/summary JSON and complete command/environment evidence are retained.
The original uncompressed files also remain in the separate local directory
`/workspace/shared/task098-uncompressed-evidence`; public verification does not
depend on that directory. No historical evidence was changed.

Source/harness identity, JSON parsing, rustfmt and repository whitespace checks
passed. Rust workspace tests/Clippy/docs were not rerun by this artifact-only task;
the coordinator's fixed-source functional gate remains distinct. The readable
report is `docs/performance-c22d8c8.md`; exact evidence hashes are under `evidence/`.

During report preparation, one `exec_command` was rejected before process start
with `exec-server transport disconnected`. The next same-environment read
succeeded and verified the files remained present. This was a transient command
connection error, not evidence of environment replacement or data loss. No
measurement was rerun because of it.

## Limits and completion

The bounded producer measurement and audit are complete. Independent artifact
review and integration/publication belong to the coordinator. The report separates
actual owner completion from receipts and named artificial waits from framework
work. CPU excludes the Python client/fixture and descendants; sampled RSS is not
private heap or a capacity claim. There is no live-provider, TUI/rendering, WSS/WAN,
large-body/history, compaction/retry throughput, soak or saturation measurement.
No latency/SLO/capacity threshold, new platform, production/harness change or
worker push was introduced.
