# Task 112 — Responses cross-model replay

## Scope and checkpoints

- Worktree: `/Users/zzq/Develops/minicore-host-wt/responses-replay`.
- Branch: `fix/0110-responses-replay`; clean starting checkpoint: `1cff421b63c04f6005288b9f304b4aac3184f3d7`.
- Baseline: SPEC v4.10.2, SHA-256 `55476e6d2c6c9ec5b857f2b9235834a162b3ad85ce56a8e2a295135f7dc370c7`; model switching/current-Turn binding and history/input projection rules reviewed.
- User's latest cross-model projection requirement supersedes earlier transparent native replay discussion. Owned paths: `crates/host-provider/src/responses/request.rs`, focused changes in `crates/host-provider/tests/responses_runtime.rs`, this task record. `PROJECT.md` remains parent-owned.
- Reference: supplied `implementations/pi-transform-messages.ts` and `pi-openai-responses-shared.ts` under `/Users/zzq/Develops/minicore-remote-build/provider-invalid-pkhm6k3c/`; accompanying `pi-revision.json` records main `c773ca2e5103f560deb71e6a5e2ae9b7d93fff92`. This is the pinned supplied reference, not a claim about a subsequently fetched latest main. Installed Pi 0.87.1 was not used as the behavior baseline.
- Requested engineering worker configuration: `gpt-6-astra`, `max` reasoning; effective runtime configuration not independently verified. No subagent facility used.

## Contract and implementation

- Cross-model means unequal replay `model` and configured `provider_model` strings, exactly; no provider, endpoint, alias or vendor inference.
- Preserve all existing original native replay/canonical validation before projecting. A model-name difference alone no longer rejects the request.
- Only after successful validation, cross-model request input drops reasoning items (including readable thinking and encrypted opaque content), message item IDs/phase (native text-signature metadata), and function-call provider item IDs.
- Ordinary text and full tool calls/results retain their order, arguments and original `call_id` pairing. Thinking is never converted to visible assistant text. No synthetic tool results; no durable-history mutation.
- Same-model native message IDs/phase, tool item IDs and encrypted reasoning are unchanged. Unknown protocol replay formats and partial formats remain explicit errors.
- No API/configuration changes, compatibility framework, core/runtime/stream/error changes, R1 unclosed-history projection or R2 cancellation changes.

## Validation and evidence

All Rust compilation/testing/formatting runs use the authorized remote runner, never local Cargo/rustfmt:

```text
python3 /Users/zzq/Develops/minicore-remote-build/responses-fixes/remote-check.py /Users/zzq/Develops/minicore-host-wt/responses-replay LABEL 'COMMAND'
```

Logs are retained under `/Users/zzq/Develops/minicore-remote-build/responses-fixes/evidence/responses-replay-LABEL.log` (completed runner invocations also have `.exit`).

| Label | Command | Actual result |
| --- | --- | --- |
| `replay-red-112-01` | `cargo test --locked -p minicore-host-provider --test responses_runtime responses_cross_model_replay` | Old production implementation: both new A→B regressions failed at target-call success assertions; exit 101. |
| `replay-green-112-01` | `cargo test --locked -p minicore-host-provider --test responses_runtime` | Both new regressions green; 21 passed, 1 failed because expanded invalid-metadata fixture omitted its tool result (`IncompleteToolExchange` before provider validation). Corrected fixture with an explicitly scripted result. Exit 101, not counted as a passing suite. |
| `replay-green-112-02` | Full validation chain below | Foreground tool timed out after 200 seconds waiting for another worktree's shared lock. No completed check result. Own orphan waiting flock PID 3287360 had no child and was terminated; rerun with a background runner to retain the eventual result. |
| `replay-green-112-03` | Full validation chain below | Exit 101. Shared-cache contamination: first test invocation incorrectly reused the stream worktree's 26-test binary (including `responses_stream_fix_*`); after test recompilation, the 22-test replay suite still linked stale Provider behavior and failed three cross-model cases. Not accepted as validation. Remote formatting copied back only intended owned-file changes. |
| `replay-green-112-04` | Final validation chain below | Exit 0. Refreshed Provider Rust source mtimes under the shared lock to force this worktree's rebuild. fmt check, clippy with warnings denied, all Provider targets (8 unit + 1 diagnostics + 42 HTTP + 22 Responses tests), 2 doc tests, and rustdoc with warnings denied all passed. |

Full validation chain:

```sh
cargo test --locked -p minicore-host-provider --test responses_runtime &&
cargo fmt -p minicore-host-provider &&
cargo fmt -p minicore-host-provider -- --check &&
cargo clippy --locked -p minicore-host-provider --all-targets -- -D warnings &&
cargo test --locked -p minicore-host-provider --all-targets &&
cargo test --locked -p minicore-host-provider --doc &&
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p minicore-host-provider --no-deps
```

Final validation chain (the shared target cache requires an explicit mtime refresh when switching worktrees; no source content is changed by `touch`):

```sh
find crates/host-provider -type f -name "*.rs" -exec touch {} + &&
cargo fmt -p minicore-host-provider -- --check &&
cargo clippy --locked -p minicore-host-provider --all-targets -- -D warnings &&
cargo test --locked -p minicore-host-provider --all-targets &&
cargo test --locked -p minicore-host-provider --doc &&
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p minicore-host-provider --no-deps
```

Local non-Rust checks: `scripts/check-whitespace.sh` could not use `mapfile` with the installed macOS Bash. Its three Git whitespace checks were run directly and passed: `git diff --check`, `git diff --cached --check`, and `git diff --check "$(git rev-list --max-parents=0 HEAD)" HEAD`. No local compilation, tests or rustfmt were run.

Coverage: captured loopback HTTP bodies for plain text A→B and reasoning/text/two complete tool exchanges A→B; canonical message serialization unchanged; encrypted-only same-model replay and cross-model omission; existing same-model native identity/phase/encrypted tests retained. Invalid metadata matrix runs for both model identities, including status, canonical text, overlapping ranges, reasoning checksum/ID, duplicate item identity, phase, tool call ID/name/count, response identity and unknown payload fields. Existing unknown-format rejection remains.

## Limits and next steps

- Loopback TCP fixtures exercise production Provider/Runtime code, not a live model service or actual external tool execution. No live model, TUI, user configuration, push or publication actions.
- Provider-scoped checks are appropriate to the owned change. Full workspace tests/lints/docs and workspace-wide unique test enumeration are not claimed here; parent integration owns combined-worktree validation. Any test totals reported here are command-local, not a workspace unique-case count.
- Full remote validation completed successfully; formatter copy-back and final diff inspected, with no out-of-scope edits. Implementation commit: `fbf5082dbc382d3fb04313ea35c042f1f788756f` (`fix(provider): project Responses replay across models`), directly on baseline `1cff421b63c04f6005288b9f304b4aac3184f3d7`. This subsequent task-only documentation update records the exact completed implementation checkpoint. Parent to integrate independently with stream/error worktrees and review; do not merge/push automatically.
- Integration runners sharing this Cargo target directory should refresh relevant source mtimes under the lock (or use an isolated target directory) before checking another worktree; locking alone did not prevent stale artifact reuse.
