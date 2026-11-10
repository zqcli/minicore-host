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

## Harness adaptation in progress

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

## Build and measurement plan

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
Compile first, then coordinate a brief no-build interval with the parent.
The execution sandbox exposes only its PID namespace to `ps`; an empty process
list is not evidence that other contributors or the machine are idle.

Planned fresh workloads are the existing bounded 8 Agents/32 Sessions stock,
32 paused inputs across 4 Sessions, 1 or 4 active model requests with 0/1/2
observers per Session, and a real bash process group cancelled under its
persistent Session owner. These counts describe fixtures, not product limits.
Begin with one smoke repetition, retain failures without summarizing them as
passing evidence, then run three fresh-Host descriptive repetitions if valid.
Results, checks, exact commits and limitations will be recorded after execution.
