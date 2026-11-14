# Task 111 — Responses stream terminal reconciliation

## Scope and checkpoints

- Worktree: `/Users/zzq/Develops/minicore-host-wt/responses-stream`.
- Branch: `fix/0110-responses-stream`.
- Baseline: `1cff421b63c04f6005288b9f304b4aac3184f3d7`.
- Implementation: `ae32f8e9e7ad34922223fe517c31e680c185e86b`
  (`fix(provider): reconcile Responses terminal stream snapshots`).
- Review input:
  `/Users/zzq/Develops/minicore-remote-build/provider-invalid-pkhm6k3c/host-review/REPORT.md`.
- Fixed references in that directory's `implementations/`:
  `pi-openai-responses-shared.ts` (Pi checkpoint
  `c773ca2e5103f560deb71e6a5e2ae9b7d93fff92`) and `agent-tools.rs`.
  These are implementation references, not permission to adopt permissive
  replacement, early tool completion, or a second argument buffer.
- Read AGENTS.md and SPEC.md provider/partial contracts (lines 508–579).
  SPEC v4.10.2 SHA-256 verified as
  `55476e6d2c6c9ec5b857f2b9235834a162b3ad85ce56a8e2a295135f7dc370c7`.
- Requested engineering worker configuration: `gpt-6-astra`, `max` reasoning;
  effective settings are not independently verified. No subagent tool was
  available in this session.

Explicit scope excludes R1 history projection, R2 Runtime cancellation,
`runtime/core`, `request.rs`, `error.rs`, real configurations/sessions, live
models and TUI. PROJECT.md is intentionally unchanged: the parent owns its
integration update. No dependency, configuration or framework was added.

## Changes

Only `crates/host-provider/src/responses/stream.rs` changes production behavior:

1. `function_call_arguments.done` checks the received prefix against the
   existing byte count/hash using UTF-8-safe slicing and emits only the missing
   suffix, once. Shorter snapshots, conflicting prefixes, invalid boundaries,
   duplicate done events, late deltas, identity/index/sequence errors and
   inconsistent later snapshots remain failures. Runtime alone accumulates and
   parses JSON; no JSON repair or additional cumulative buffer is introduced.
2. R3: a final reasoning snapshot may enrich absent `encrypted_content` with
   an actual nonempty value. All other consumed fields and identity must match;
   an existing cipher cannot change or disappear. After validating all final
   output snapshots, retain the final native reasoning item for replay,
   including encrypted-only responses without invented text.
3. R4: message/function-call items closed with `status=incomplete` can await
   the real terminal. Their received content/arguments and identity still must
   match; subordinate done frames need not exist for incomplete items. Remember
   the incomplete fact so a later `response.completed` cannot turn it into a
   success, even if its snapshot rewrites the status. The existing failure path
   hands off actual partial data, terminal reason and usage without ToolCallEnd.

`crates/host-provider/tests/responses_runtime.rs` has six appended tests only;
existing helpers/tests are unchanged to keep concurrent integration focused.
They exercise production HTTP/SSE decoding and the real Runtime ModelDriver
using explicitly scripted loopback fixtures, not live provider responses:

- Missing suffix: no deltas, added-item prefix plus delta, multibyte prefix,
  already-complete initial arguments; exact emitted fragments and one tool end,
  plus Runtime JSON result.
- Guards: prefix conflict, shorter snapshot, UTF-8 boundary, identity/index,
  duplicate done, late delta, sequence, item/final snapshot conflict, EOF and
  invalid JSON; original/accepted partial argument text remains exact.
- R3: summary and encrypted-only enrichment reaches the next native request;
  conflicting/removed/empty encryption and identity/summary/status changes fail.
- R4: message and truncated function-call items, with and without subordinate
  done events, retain terminal usage/reason and non-executable partials;
  completed terminals cannot conceal known incomplete items.

## Remote validation — 2026-11-14

All Cargo, compilation, rustfmt, Clippy and Rust documentation commands ran
only through this runner:

```sh
python3 /Users/zzq/Develops/minicore-remote-build/responses-fixes/remote-check.py \
  /Users/zzq/Develops/minicore-host-wt/responses-stream LABEL 'COMMAND'
```

It syncs to `/root/mct/responses-fixes/responses-stream`, serializes commands
under `/root/mct/fixes-0110/cargo-host.lock`, uses the shared
`/root/mct/fixes-0110/target-host`, and copies formatted crates back.
Evidence directory:
`/Users/zzq/Develops/minicore-remote-build/responses-fixes/evidence/`.
Each log is named `responses-stream-LABEL.log` (and `.exit` when the runner
finished). Exact label/command/results:

### Red before production edits

`stream-red-01`:

```sh
cargo test --locked -p minicore-host-provider --test responses_runtime responses_stream_fix_
```

Exit 101: six new tests ran against unchanged baseline production code;
four failed, two defensive tests passed. The three positive regressions failed
on done-only arguments, final-only encryption and premature incomplete-item
rejection. The argument guard matrix also failed because the old code rejected
the first valid suffix before reaching the duplicate-event guard. This was an
actual test failure, not a build failure. Production code was edited only after
this red run; no stash/reset or history rewrite was used.

### Focused green

`stream-green-01` and `stream-green-02` used the same command:

```sh
cargo fmt --all && cargo test --locked -p minicore-host-provider --test responses_runtime responses_stream_fix_
```

The first invocation exceeded the local 200-second tool wait and has no final
runner result; it is not a claimed pass. No matching local runner/SSH process
remained when inspected. The second invocation used a longer wait, returned
exit 0, and passed all six tests. Later clean rebuilding independently verified
all six again.

### Full provider gates and shared-cache correction

`stream-provider-01`:

```sh
cargo fmt --all -- --check && cargo test --locked -p minicore-host-provider && cargo clippy --locked -p minicore-host-provider --all-targets -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc --locked -p minicore-host-provider --no-deps && ./scripts/check-whitespace.sh
```

Exit 101: the shared target reused a Responses test binary from another
worktree (22 cases, including cross-model test names absent from this source,
instead of this worktree's 26 cases). Three foreign cases failed. Local source
and diff remained unchanged; this is not treated as a regression result or a
valid full-suite pass. To invalidate that stale package artifact under the
same remote build lock, the next invocation cleaned only the provider package.

`stream-provider-02`:

```sh
cargo clean -p minicore-host-provider && cargo fmt --all -- --check && cargo test --locked -p minicore-host-provider && cargo clippy --locked -p minicore-host-provider --all-targets -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc --locked -p minicore-host-provider --no-deps && ./scripts/check-whitespace.sh
```

Confirmed clean compilation from this worktree, followed by:

| Gate | Actual result |
| --- | --- |
| Workspace rustfmt check | Passed |
| Provider unit tests | 8 passed |
| Provider diagnostics integration | 1 passed |
| Provider HTTP/Runtime integration | 42 passed |
| Provider Responses/Runtime integration | 26 passed, including all six new cases |
| Provider doctests | 2 passed |
| Provider Clippy, all targets, `-D warnings` | Passed |
| Provider rustdoc, no dependencies, `-D warnings` | Passed |
| Remote whitespace script | Blocked: runner deliberately excludes `.git` |

The combined runner exit is 1 solely from that last whitespace script, after
all Rust gates above completed successfully. The script's exact Git checks
were then executed locally, without Cargo: non-shallow history, exactly one
root, `git diff --check <root> HEAD`, `git diff --check --cached` and
`git diff --check`. All passed, as did the staged implementation diff check.
No workspace-wide unique test count is claimed: a workspace `--list` and
workspace-wide tests/lints/docs were not run for this provider-only change.

## Delivery and limits

Implementation is locally committed; this task record is a separate follow-up
documentation commit so it can name the exact implementation SHA without
rewriting history. No push, publication, merge, real model call, TUI launch or
real-session/configuration change occurred. No local Cargo/compilation/rustfmt
was run. No macOS/Windows runtime validation or Session persistence integration
is claimed: these tests prove the provider/Runtime partial handoff, not a new
storage acceptance test.

Original handoff: integrate this implementation and task record alongside
the separately owned replay/error fixes, update PROJECT.md, review the combined
test-file changes, and run integrated acceptance. Because remote worktrees share
a Cargo target, invalidate the provider package artifacts under the runner lock
when switching source checkpoints; do not accept stale test names as evidence.

## Independent-review follow-up — R4 reasoning, 2026-11-14

The owner reports the previous commits were integrated into the dev checkout at `5215f33`,
alongside separately owned request/error changes. This follow-up stays on the
original topic worktree at `14104926fe972b42ba50915fd2d01a06f92c70ae`; it does not
merge main or those unrelated changes. The original R4 repair covered only
message/function-call items and missed reasoning item closure.

Follow-up implementation commit:
`b7b1ef1346187e6856515244dc3c2601cb2fc28f`
(`fix(provider): retain incomplete Responses reasoning items`).
This task update is a separate new documentation commit; neither commit amends
or rewrites the already integrated history.

### Minimal repair

- `responses/wire.rs` splits reasoning structure validation from the existing
  strict complete-replay validator. `validate_reasoning` still accepts only
  absent/completed status and delegates unchanged structural checks. Its
  signature and `request.rs` call site are unchanged.
- `responses/stream.rs` explicitly permits absent/completed/incomplete status
  at reasoning item closure. Summary and reasoning-content snapshots may lack
  subordinate done frames only for an incomplete item; identity, part counts,
  kinds and received-text fingerprints are still checked. The native item is
  retained through the existing partial replay path.
- `response.completed` still uses strict validation and the existing
  `output.incomplete` guard. Keeping or rewriting the final item status cannot
  convert known incomplete reasoning into success. No Runtime/core, R1/R2,
  request/error, PROJECT.md, dependency, real configuration or session changes.

Four appended integration tests and one fixture helper cover:

1. Empty summary, streamed summary with done frames, and streamed summary
   without text/part done frames. Each reaches the real `IncompleteResponse`,
   retains actual usage, `incomplete_details`, and the exact native reasoning
   item (including incomplete status and encrypted content). Driver attempts
   and fixture request counts prove no retry. Direct provider-stream inspection
   verifies replay/usage/error and no ToolCallEnd/Finish even with a sibling
   protocol-closed function call; Runtime keeps that call non-executable.
2. For each summary shape, a subsequent `response.completed` is rejected both
   when the final item remains incomplete and when it changes to completed;
   partial replay retains the original incomplete native item.
3. Identity, text, content-kind and invalid-status conflicts plus duplicate
   item.done still fail without replacing actual streamed reasoning text.
4. An incomplete reasoning item inserted into complete replay is rejected
   before network dispatch; complete-history policy was not globally relaxed.

### Red/green and full validation

All Rust commands used the same remote runner invocation and evidence directory
documented above. The updated runner now touches all `host-provider` Rust source
files inside the shared lock before each command, preventing reuse of another
worktree's stale provider artifact. Each run below visibly compiled this source.

`reasoning-r4-red-01` (before production edits):

```sh
cargo test --locked -p minicore-host-provider --test responses_runtime responses_incomplete_reasoning_
```

Exit 101: 2 failed, 2 passed. The positive partial test received
`InvalidProviderResponse` instead of `IncompleteResponse`; the completed-disguise
test found no retained native incomplete item. Structural/replay guard tests
passed. This is an actual regression red, not a compilation failure.

`reasoning-r4-green-01`:

```sh
cargo fmt --all && cargo test --locked -p minicore-host-provider --test responses_runtime responses_incomplete_reasoning_
```

Exit 0: all 4 tests passed, including every summary/status matrix branch.

`reasoning-r4-provider-01`:

```sh
cargo fmt --all -- --check && cargo test --locked -p minicore-host-provider && cargo clippy --locked -p minicore-host-provider --all-targets -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc --locked -p minicore-host-provider --no-deps
```

Exit 0: formatting passed; 8 provider unit, 1 diagnostics, 42 HTTP/Runtime,
30 Responses/Runtime tests and 2 doctests passed; all-target provider Clippy
and provider rustdoc with warnings denied passed. The staged diff whitespace
check and local full-history whitespace checks also passed. No local Cargo,
compilation, tests or rustfmt was used. No new workspace-wide count or
workspace-wide/native-platform/live-model/TUI/storage acceptance is claimed.

The follow-up required no request.rs signature adjustment. No push or
publication was performed.

## Integrated acceptance

The combined dev code checkpoint is `35873c69b306b7aafdffa8c461ab9dd40d111625`.
Independent review accepted the incomplete-reasoning follow-up. Remote Linux
`scripts/check.sh` passed 1,196 unique Rust tests, 7 doctests, 28 Python checks,
whitespace, formatting, strict Clippy and strict rustdoc. The unique count comes
from a separate `cargo test --workspace --all-targets --locked -- --list` run.
[Integration evidence](integration.json) records commands and environment limits.
R1/R2, native macOS/Windows acceptance, live models, installation and publication
remain outside this repair; the installed binary and TUI pin are unchanged.
