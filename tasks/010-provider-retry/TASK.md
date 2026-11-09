# Preserve streamed rejection retry hints

## Scope and exact baseline

- Review regression repair on 2026-11-09 (UTC), from Host `6d29cdca4c265c3f1eaee906d97f7855c9d74545`.
- Published Runtime remains `18a6ac9b82d003330cd870275efca793cdcc9a96` (`0.9.2`). No local Runtime patch, dependency version change, or lockfile change.
- Owned production files: `crates/host-provider/src/error.rs` and `src/stream.rs`; supporting tests, provider README and this task record. The integrator explicitly approved adding only Tokio `test-util` to the provider dev-dependency features.
- SPEC v4.93 §11: ModelDriver alone retries. A valid Retry-After is a lower bound; a remaining wait over 30 seconds or beyond the original deadline stops retries. Unknown/Started delivery, observed events, hard quota and authentication failure cannot become retryable merely because a header exists.
- Real HTTP means the actual reqwest adapter sends to a scripted loopback TCP server. The actual pinned AgentLoop/ModelDriver executes the call and decides retries. These fixtures are not a live provider or substitute Runtime.

## Reproduced failure

The successful HTTP response was consumed into an SSE byte stream before its status and retry headers were retained. The first `rate_limit_error` frame therefore produced `Retryable { retry_after: None }`, even with `Retry-After: 60`.

Test-only commit: `dc14afd99dfab9854a688947aa74b4f29419fc07`.
Its production provider source is identical to baseline `6d29cdc`.

At that commit, this command **failed**:

```sh
cargo test --locked --offline -p minicore-host-provider --test http_runtime \
  sse_long_retry_after_prevents_runtime_from_retrying_early -- --exact --nocapture
```

The fixture recorded **two real HTTP requests 3.55839 ms apart**, versus the required one request. An earlier run of the same regression measured 3.939055 ms. The model timeout is deliberately 120 seconds, so the 60-second hint fits the deadline but must be rejected by Runtime's 30-second retry-wait limit. The regression is not merely testing an insufficient call deadline.

The earlier 198 workspace tests plus 2 doctests passing did not cover this HTTP-200/SSE-header path and did not establish whole-foundation review acceptance.

## Repair

Fix commit: `f5547612636239db16ae7bf5a59ab4b99476381f`.

- Retain HTTP status and parsed absolute retry time before consuming a successful response's headers.
- Pass that evidence to structured SSE rejection classification. Only recognized temporary, pre-event rejections get the remaining retry lower bound.
- Preserve the status in bounded diagnostics. Unknown, quota, authentication and Started failures remain non-retryable. A role-only choice still counts as Started, and real partial text remains intact.
- Reuse the existing header parser, including seconds, milliseconds and HTTP-date handling. The clock is not restarted after body parsing, and the provider does not cap the value.
- Add no provider/Host/Agent/Session/tool retry layer, waiting policy, implicit replay or Runtime modification. Primary outcome, partial output and history-saving ownership are unchanged.

## Regression coverage

Nine new real HTTP/SSE + AgentLoop tests cover:

1. HTTP 200 + first-frame rate-limit rejection + `Retry-After: 60`: one request, structured RateLimited/NotStarted failure, retained hint and HTTP status, no assistant success or partial output.
2. Shorter seconds and millisecond hints, invalid-millisecond fallback to valid seconds, and exponential-backoff precedence: actual TCP request timestamps meet the lower bound, the request body is identical, and Runtime reports one logical request despite two physical attempts.
3. Absent, invalid, negative, non-finite and expired hints: the real adapter reports no valid delay; Runtime still applies its own bounded backoff.
4. Unknown/free-text-only classification, hard-quota precedence, authentication and invalid-request errors: a header grants no retry permission.
5. Role-only and text output before rejection: Started/Never, one physical request, original partial text retained and no fabricated assistant completion.
6. Model timeout and earlier Turn deadline that cannot fit the valid hint: one request and the original structured rejection.
7. Cancellation inside the actual retry wait: observe the real provider's error without altering it, pause the monotonic clock, advance one second of a ten-second wait, verify one TCP request, cancel, and obtain the real User-cancelled outcome.
8. A late retry wakeup after the original model timeout: controlled monotonic advancement crosses both timers; Runtime preserves the exact original ModelError and sends no second request.
9. A late retry wakeup after the original Turn deadline: the same controlled advancement yields the real Deadline-cancelled outcome without a second request.

The observer used for cases 3 and 7–9 is a named, test-only pass-through around the real provider. It forwards each request/context and exact result unchanged; it neither scripts a model response nor retries. Clock control starts only after the real TCP response was parsed, preventing virtual time from advancing ahead of pending network I/O. No guessed sleeps stand in for a correctness assertion.

## Validation

Environment: Rust `1.90.0`; `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, Cargo on PATH, `TMPDIR=/workspace/shared/minicore-host-tmp`, dedicated `CARGO_TARGET_DIR=/workspace/shared/minicore-host-provider-retry-target`.

From the workspace root, with the unchanged committed root lock and published Runtime Git source:

- The exact regression command above: **PASS after the repair**.
- `cargo fmt -p minicore-host-provider -- --check`: **PASS**.
- `cargo clippy --locked --offline -p minicore-host-provider --all-targets -- -D warnings`: **PASS**.
- `cargo test --locked --offline -p minicore-host-provider`: **PASS**, 5 unit + 31 integration + 1 compiling doctest.
- `RUSTDOCFLAGS='-D warnings' cargo doc --locked --offline -p minicore-host-provider --no-deps`: **PASS**.
- `git diff --check`: **PASS** for this working change set.

## Handoff and limits

The root integrator owns PROJECT updates, final combined workspace validation, the separate pre-existing manifest whitespace fix and committed-diff gate, and independent review. These provider-scoped results do not close other controller, retirement or save-state review findings.

No live provider or TUI was called, no upstream dependency or protected baseline was edited, and no remote write or push was attempted. The owner must separately approve any eventual push to this repository.
