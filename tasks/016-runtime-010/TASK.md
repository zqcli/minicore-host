# Task 016: Adopt Runtime 0.10

## Scope and isolated baseline

- Host baseline: `8079970d3d7b35912ae4b45118d8e980d5f3e749`.
- Branch/worktree: `topic/runtime-010-integration` at `/workspace/shared/minicore-host-runtime-010`.
- Candidate Runtime: version `0.10.0`, local commit `cd59b6ecbad5ccfb67552660290992662253219b`, tree `a9affb5ec0a2c40dc68c77c1e0dec1a13bd2cc49`, directly based on published `18a6ac9b82d003330cd870275efca793cdcc9a96`.
- Runtime 0.10 is not published when this task starts. No manifest or lockfile may claim that this local commit is remotely fetchable. Source commits require the complete migration and a separately finalized dependency pin before integration.
- Work is local only. Do not change Host main/dev, sibling worktrees, Runtime, Agent or TUI; do not push or merge main.
- Read `AGENTS.md`, unchanged `SPEC.md` Rule 12, Runtime's migration notes and `scripts/check.sh` before editing.

## Planned commits

1. Mechanical `Text` and infallible model-event API adoption across Host source/test adapters.
2. Remove obsolete provider argument/error-body quotas and event splitting. Preserve provider fragments, invalid-JSON checks, identity grammar, cancellation, deadlines and conservative retry behavior.
3. Real loopback HTTP/SSE to actual Runtime and Host-owner regressions beyond the former byte limits.
4. Record local validation. A dependency revision/lock commit remains a separate publication-dependent integration step.

These are review units, not claims that the old Runtime pin builds each intermediate source commit.

## Temporary local validation contract

The real candidate Runtime source is `/workspace/shared/minicore-runtime-unbounded`. Validation will use an explicit external Cargo patch and temporary manifest versions, not a committed `.cargo` file, vendor copy or substitute Runtime. The selected package's version, manifest path and source must be checked with `cargo metadata` before reporting tests.

Required gates: whitespace, format, workspace/all-target Clippy with warnings denied, workspace/all-target tests, workspace doctests and rustdoc with warnings denied. Count unique test cases from `cargo test --workspace --all-targets --locked -- --list`; nested storage subprocess runs are not additional cases.

## Status

Mechanical API edits are in progress. No compilation, tests or documentation gate is yet claimed. Dependency publication and final reproducible Git pin remain pending separately.
