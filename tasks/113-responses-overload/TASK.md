# Task 113 — Host review R5: documented overload classification

## Status and checkpoints

- Implemented and provider-scoped validation passed on `fix/0110-responses-errors`.
- Baseline: `1cff421b63c04f6005288b9f304b4aac3184f3d7` (initial index/worktree clean).
- Implementation: `4114de3a482bff945a8e33386a29831e1ba46c34` — `fix(provider): recognize documented server overload errors`.
- This task record is a separate documentation commit following the implementation; no history rewrite or push.
- Requested engineering-worker configuration per AGENTS: `gpt-6-astra`, `max` reasoning. No subagents were available/requested, and effective runtime settings were not independently verified.

## Scope and evidence

Read `AGENTS.md`, SPEC.md's provider contract and finite-retry/deadline section (2098–2102), the actual shared error classifier, HTTP fixtures, and ModelDriver retry ownership. Review inputs:

- `/Users/zzq/Develops/minicore-remote-build/provider-invalid-pkhm6k3c/host-review/REPORT.md`, R5.
- `/Users/zzq/Develops/minicore-remote-build/provider-invalid-pkhm6k3c/host-review/openai-error-codes.md`, supplied official-page excerpt documenting HTTP 503 with `type=service_unavailable_error`, `code=server_is_overloaded`.

Only production change: add `server_is_overloaded` to the existing `structured_kind` overload identifiers in `crates/host-provider/src/error.rs`. Reuse existing code/type scanning, quota precedence, status gates, started-delivery handling, and ModelDriver policy. The precise overload identifier is sufficient with omitted or unfamiliar generic type; requiring the exact pair would unnecessarily reject compatible providers. The generic `service_unavailable_error` type alone, or with an unknown code, remains insufficient evidence for safe delivery. Matching message text is not allowed.

Direct regressions in `crates/host-provider/tests/http_runtime.rs` prove:

- Official HTTP 503 pair succeeds through the real provider and ModelDriver with exactly two identical requests.
- Code-only and different generic type retain compatibility.
- Separate Retry-After seconds / milliseconds lower bounds and Runtime backoff are respected using captured TCP request timestamps.
- Exhausted attempt budget, insufficient original model deadline, and the 30-second wait cap each prevent a second request, preserving the recognized error and retry hint.
- Unknown 503, generic type alone, message-only overload identifier, and HTTP 500 with the precise identifier do not authorize retries (`Unknown / Never`, one request).

Inline regression checks the new identifier cannot override started SSE delivery or hard quota precedence, and preserves the existing code/type field convention. Full existing provider tests additionally cover role-only/partial SSE, quota/auth/invalid errors, cancellation, turn/model deadlines, and Responses initial/started errors.

## Remote verification

All Cargo, compilation, tests and rustfmt ran remotely via:

```text
python3 /Users/zzq/Develops/minicore-remote-build/responses-fixes/remote-check.py /Users/zzq/Develops/minicore-host-wt/responses-errors LABEL 'COMMAND'
```

Logs are under `/Users/zzq/Develops/minicore-remote-build/responses-fixes/evidence/`, named `responses-errors-LABEL.log`; completed runner invocations also have `.exit` files.

1. **Red**, label `r5-red-01`, exit 101, tests added before production fix:
   ```text
   cargo test --locked -p minicore-host-provider --test http_runtime official_overload_http -- --nocapture
   ```
   Two new positive tests failed because the exact official pair produced `ProviderUnavailable / Unknown / Never`; unknown-delivery negative regression passed. This was a behavior failure, not compilation failure.

2. **Green**, label `r5-green-01`, exit 0:
   ```text
   cargo test --locked -p minicore-host-provider official_overload -- --nocapture
   ```
   All four direct regressions passed (three HTTP tests, one inline test).

3. **Complete provider checks**, label `r5-checks-01`, exit 0:
   ```text
   cargo fmt -p minicore-host-provider && cargo fmt --all -- --check && cargo test --locked -p minicore-host-provider --all-targets && cargo clippy --locked -p minicore-host-provider --all-targets -- -D warnings && cargo test --locked -p minicore-host-provider --doc && RUSTDOCFLAGS="-D warnings" cargo doc --locked -p minicore-host-provider --no-deps
   ```
   Results: unit 9 passed; diagnostics 1 passed; HTTP 45 passed; Responses 20 passed; doc-tests 2 passed. Clippy, workspace fmt check and provider rustdoc passed. Remote formatting copied back only changes in the two owned Rust files. These are provider harness results, not a workspace-wide unique-case total.

4. **Workspace enumeration incomplete**, label `r5-list-01`:
   ```text
   cargo test --workspace --all-targets --locked -- --list
   ```
   Attempted the AGENTS-prescribed inventory command. The local tool wait timed out after 200 seconds during remote workspace compilation; the runner did not produce an exit file or complete test list. Read-only process checks confirmed the detached remote Cargo process continued compiling. To release the shared target lock, terminated only this invocation's verified process group (PGID 3286233); subsequent polling confirmed `R5_TIMED_OUT_LIST_GROUP_STOPPED`. No completed workspace unique-test count or pass is claimed. Other contributors' process groups were not touched.

Local verification was limited to source/diff inspection and `git diff --check` (passed); no local Cargo, compilation, tests or rustfmt.

## Limits and integration handoff

- Fixtures use a scripted loopback TCP HTTP server, real reqwest/provider and ModelDriver. This is not live model/provider or TUI validation.
- No Runtime retry strategy, stream/request/core code, `responses_runtime.rs`, `PROJECT.md`, manifests or lockfile changes. `PROJECT.md` integration belongs to the parent.
- Dual Retry-After header precedence is explicitly out of scope and unchanged.
- Full workspace execution, workspace-wide clippy/docs and completed unique-test enumeration were not performed; provider checks above are complete. Parent integration may run broader checks and the inventory after combining disjoint review fixes.
- No real configuration/session access, model traffic, TUI launch, push or publication.
- Next: parent independent review/integration of the implementation and task-record commits, preserving history.
