# Foundation review handoff

## Review target

Review the published `dev` commit [`82583b31ac9091164da5f6dcfee6bb23e05137e5`](https://github.com/zqcli/minicore-host/commit/82583b31ac9091164da5f6dcfee6bb23e05137e5), compared with `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. It reproduces local `797622fa9d663db01fad0d8b800a51a181ddc150`, whose code checkpoint is `275110695551c30582d1302e6b01ab23a3aad536`, followed only by handoff documentation. This is the current `dev` foundation, **not a complete v4.93 release**. Read `AGENTS.md`, complete `SPEC.md`, `PROJECT.md` and `docs/method-coverage.md`. The unchanged specification SHA-256 is `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

The owner authorized the dev handoff. Remote `main` remains the README-only `b341b163e14543bfbe7f8d392ea243b4a123cdf7`, preserved as an ancestor of the published dev history. The [complete 101-commit publication mapping](host-publication.json) translates local source SHAs in task records to available GitHub commits. Each original tree and raw message was verified; default GitHub commit authors and committers are `zqcli`, with no co-author trailers. No main merge is authorized by this handoff. External independent review and required gates remain prerequisites.

Review must not depend on another conversation, an unpublished Runtime worktree, hidden test data or a prior review's conclusion. The 52-method catalog describes contracts, not 52 finished handlers; 25 primitive routes currently share typed/stdio/WSS dispatch.

## Dependency identity

- Runtime `0.10.0`: published [`59ee8ce668f72a21d408e6247d3ce075b260db14`](https://github.com/zqcli/minicore-runtime/commit/59ee8ce668f72a21d408e6247d3ce075b260db14) on `feat/host-history-handoff`. All four manifests and the sole lockfile use it. There is no local patch, copied source or bootstrap dependency.
- Runtime tree `a9affb5ec0a2c40dc68c77c1e0dec1a13bd2cc49` equals the original local `cd59b6e` tree. [Commit mapping and verified publication facts](runtime-publication.json) are included. [Exact-head CI](https://github.com/zqcli/minicore-runtime/actions/runs/42619794415) passed all seven checks.
- This includes authoritative history handoff, partial reports, separate persistence failures and removal of arbitrary application quotas. It retains grammar, parser stack safety, real model limits, cancellation/deadlines and finite retry. The separate local retry-progress interface is not in this dependency or Host checkpoint.
- Agent `f36a87366d8e2fe0a1c8216463887902e471c739` and TUI `3beeb1d8a525e58ce6352e05061f242c5580e248` are unchanged. Both lock Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`; no joint integration with their current protocol is claimed.

## Paths worth tracing

1. Protocol: strict IDs/direct owners, text inputs, configuration/patch validation, creation/control receipts, known Fault schemas and lossless unknown Fault leaves. Check invalid combinations and misleading availability separately from routing.
2. Storage: Linux directory-inode ownership, fd-relative no-follow operations, independent discovery, exact stored identities and native JSONL. Readable bytes or accepted history must not be mislabeled confirmed persistence.
3. Execution: Host -> Agent -> SessionController -> real Runtime. Trace both input queues, configuration/Turn binding, same-Turn continuation, cancellation claim, finalization and separate primary/save failures. Persistent model content does not come from public observation events.
4. Persistent owners: asynchronous creation/update outlive callers, partial receipts retain published facts, metadata confirms before publication, history loads lazily, close/default dependencies and root leases remain owned. A failed child must not block unrelated siblings.
5. Lifecycle: immediate canonical control facts, monotonic graceful/force shutdown, failed-save blocking, explicit Session-only original-save continuation, actual joins, lease release and memory reclamation. Caller/connection abandonment does not abandon accepted work.
6. Provider: production Chat Completions HTTP/SSE, conservative retry/delivery classification, Retry-After lower bounds, real model/tool replay, and incomplete headers retained as non-executable partial metadata. The earlier SSE Retry-After bug has a permanent real-HTTP regression.
7. Transport: shared typed/stdio/WSS dispatch, real TLS/authentication before Host registration, first-valid initialization rights, read-only connections, key-change revocation, one wait budget and unknown nested Fault preservation. This remains an embedding API, not a production process/configuration loader.
8. Workspace: real configured-root file/search/Git reads and explicit edit snapshots. Clean/process filters are rejected when applicable; unsafe repository errors are not reported as clean/non-repository success. Canonical Session routing and actual write-tool capture remain open.

## Reproducing checks

Use Linux, pinned Rust 1.90.0, rustfmt and Clippy with full Git history:

```sh
cargo metadata --locked --format-version 1
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
sha256sum SPEC.md
```

At code checkpoint `2751106`, the full gate passes **340 unique tests and 3 doctests**, committed/staged/working whitespace, formatting, all-target Clippy and rustdoc with warnings denied. Count one `: test` line per listed test; nested storage subprocess output is not another case. The same complete gate passed on Ubuntu in [exact-head CI run 42629644954](https://github.com/zqcli/minicore-host/actions/runs/42629644954) for the published commit. Cached Cargo downloads do not change the fixed Git dependency identity.

Evidence boundaries:

- Real AgentLoop and owners with explicitly scripted/gated models; real filesystem save barriers, failures and partial reconciliation. These are not fake Runtime tests.
- Real loopback TCP/HTTP/SSE through the production provider, including multiple Host Turns, large payloads and truncated-response partials.
- Real pipes and loopback TLS/WebSockets, fixture certificates, operation-right revocation and actual SDK/server framing beyond 16 MiB.
- Real storage child processes for inode locking, competing instances and abrupt termination; real Git/file queries and edit observations.
- No live model account, production Host process, TUI session or performance/capacity measurement is claimed. Repeated concurrency tests are not capacity evidence.

## Review history and corrections

Exact commands, failures, source-to-dev mappings and historical counts remain in `tasks/*/TASK.md`:

- Tasks 001/006: initial foundation, clean-source gates, authoritative save barriers and owner corrections.
- Tasks 010/011: HTTP 200 SSE `Retry-After: 60` originally lost its lower bound. `7b24ca2` fixes it, with real HTTP/Runtime negative and positive regressions. Earlier passing tests had missed this case.
- Task 012: controlled shutdown/save continuation/memory retirement, concurrent-close correction and explicit first-version limits.
- Task 013: canonical control receipts; independent review found lookup absence/ambiguity conflated with unsupported methods. `b961f9f` fixes this with distinct Faults and a 161-case scoped recheck.
- Task 014: workspace backend and Git filter/damaged-repository corrections. Independent scoped verification passes under stable local Git configuration/attributes; no protection against uncoordinated external mutation is claimed.
- Tasks 015/017: typed/stdio/WSS integration. Review found unknown Fault rejection and synchronous creation validation blocking client deadlines. Corrections integrated through `5921c72` pass the full **331-test/3-doctest** gate. Separate review reran the original five probes, added four real WSS probes and repeated 29 focused cases three times, with no remaining scoped finding. It also checked TLS rejection, network revocation and owner lifetime.
- Task 016: Runtime 0.10 source migration, removal of provider argument/error-body quotas, nine new real HTTP/Host regressions and the actual published dependency pin. Two temporary old-limit mutations failed the new tests; restored sources passed. Integrated checkpoint `2751106` passes **340 tests and 3 doctests without a patch**.

A workspace integration attempt once hit the existing storage test's `HistoryInUse` failure; a focused rerun and subsequent full gates passed, but the cause was not established and it is not described as fixed. Other failed attempts and environmental disk/link limits are retained in task evidence rather than converted into passing results. Scoped reviews are useful evidence, not the requested full-product independent acceptance.

## Required work still open

- Remaining canonical contracts and routes among all 52 methods, phased persistent deletion, complete execution-resource configuration/reload and actual Host process assembly.
- Full scoped/filterable observation routes and body/history watermarks, DSH delegation and built-in tools, actual workspace write capture, standard interactions, compaction and old incomplete-tool-history projection.
- Live authorized provider and actual TUI validation in later stages, performance measurements and external independent review.

Topic branches for ongoing DSH/observation work are not part of this checkpoint. Do not treat their local tests, DTOs or fixture ports as completed integrated behavior. Findings should identify the exact commit, violated contract and reproducer, distinguishing implemented-path defects from declared unfinished surfaces without waiving either for the final release.

## Explicit local-use limits

No new recovery entry, witness layer or automatic repair exists for an unpublished creation's failed necessary save. Such a failure cannot become successful stopped shutdown; repeated shutdown returns facts and does not retry. Existing Session resume confirms only its own retained originals.

Dropping the last root Host after an already-failed save without controlled shutdown is uncontrolled destruction, not normal stopped retirement. No permanent self-cycle is introduced. Ordinary caller abandonment and accepted explicit shutdown still retain real owners.

Workspace Git preflight assumes stable local configuration/attributes. Uncoordinated changes between preflight and Git execution remain a known limitation; no atomic snapshot or cross-process lock is claimed.
