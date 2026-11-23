# Task 122: Long-run usability fixes in Host/TUI 0.12.11

## Scope and baselines

The owner confirmed the findings of a long-running TUI test (three context
compactions, injected upstream faults, parallel delegation and a tool self-test)
and requested fixes in both repositories with one shared version, 0.12.11.
This task prepares local commits only: no push, tag, merge or release.

- Host `dev`: `e05bb4b890f79ccf51ed1f5f76dc6f2e46e74ecc`, version 0.12.9.
- TUI `dev`: `86cad36a473ee4235120d282339e2e15c080a171`, version 0.12.10.
- SPEC advances from 4.11.5 to 4.11.6; AGENTS.md records the new SHA-256.

## Changes

- `fix(runtime)` f1499bb: the model driver retries transient failures (any
  429, 408/425/5xx gateway statuses, transport and unknown-outcome faults, a
  stream interrupted before output) within the configured attempts while no
  text or reasoning was forwarded and no usage was observed. A 429 is a
  rate-limit rejection whatever its body and honours Retry-After. Auth, quota,
  invalid request, context overflow and cancellation stay final.
- `fix(compaction)` 4bf9a8f: when automatic compaction runs between tool
  calls and the newest user input falls inside the summarized prefix, the
  summary carries that input verbatim plus an instruction to continue the work
  in the request's language. Idle manual compaction is unchanged.
- `fix(tools)` cf586a9: local business failures are `failed` results with the
  same structured body; argument errors name the missing field or type
  constraint without echoing model-supplied names; `read` defaults `limit` to
  2000; the model copy of a bash stream over 32 KiB keeps 16 KiB from each end
  with `omitted_bytes` and a visible marker while owner records keep every
  byte; a timed-out bash returns its partial output; edit and apply_patch
  explain no-match and new-file cases; Runtime failures state their reason.
- `feat(agent)` 4043ae6: `create` targets default to the calling Host/Agent; a
  new Agent copies the caller's Agent unless `from_agent_id` names another;
  delegated Sessions inherit the caller Session's execution policy and
  workspace (and model/reasoning for the same Agent); untitled delegated
  Sessions are titled from the task's first line; unknown-field diagnostics
  list the fields the action's own schema branch accepts.
- `docs(spec)` ba26d8f: SPEC 4.11.6 records these rules.
- `chore(release)`: fourteen packages, internal requirements and lock records
  become 0.12.11; current README, CI and configuration references follow.

## Validation

On Linux (Debian 12 container) with Rust 1.90.0:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --all-targets --locked --no-fail-fast`: 1,269 passed,
  1 failed. The failure is
  `local_binding::cancelled_real_bash_is_waited_before_finished_and_same_call_id_is_turn_scoped`,
  which runs `/bin/kill`; this container has no procps and no root, so the test
  environment, not the change, fails it. CI runners provide `/bin/kill`.
- `cargo test --workspace --all-targets --locked -- --list`: 1,274 cases.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`: passed.
- `./scripts/check.sh` (Python checks) was not run: the container has no Python.

No remote CI, macOS/Windows execution, platform packaging or live-provider
acceptance is claimed by this local gate.

## Publication order

After separate authorization, push this Host commit first, verify its remote
SHA, then push TUI with the same immutable Host pin. If publication changes the
Host commit, update the TUI manifest, lockfile and changelog pin and repeat the
TUI checks. Do not force-push or rewrite remote history.
