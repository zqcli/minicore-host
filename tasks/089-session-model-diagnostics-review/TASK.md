# Task 089: Independent Session and model diagnostics review

## Scope and fixed sources

This review is read-only for production code. It covers the scheduling, first
explicit cancellation and actual asynchronous Closed additions in
`d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06..f36761a164c6f3ae092c5e0bbf7ab08bbcd58891`:

- `ab29ae87c8dfe2d06720fd43cc15dda899bebbf5`
- `f36761a164c6f3ae092c5e0bbf7ab08bbcd58891`

It also reviews the model attempt/usage consumer already integrated as
`9cbf426a54ce68bb43128b62f25dfa2320f60305`, plus the isolated elapsed-wait
consumer `e4be354c8067690e399e8dbf56dfeb495be74fac` against Runtime
`2f7aeb9a2112e69cf1918c375916656e54d1208e`. The original model helper bytes
are identical between the current Host source and the candidate's parent
(SHA-256 `8f3a9eff9d5318821f0149287da1b740f89203d5837c646f53d9e122777ab795`).
The elapsed consumer is not a dependency publication or approval to publish the
Runtime change.

SPEC v4.93 remains the baseline, SHA-256
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Task 053 and the direct-owner implementation were read before testing.
minicore-agent is reference material, not a compatibility baseline. TUI, public
client events, production dependencies, main and remote refs are unchanged.
The separate saved-turn-result restart defect and task087 fix are outside this
review. Requested engineering configuration: `gpt-6-astra` with `max` reasoning;
effective model/runtime settings cannot be independently attested by this worker.

## Isolation and provenance

Three detached source worktrees are used, each with its own target directory:

- Host `minicore-host-task089-review`, target `task089-target-host-baseline`.
- Candidate `minicore-host-task089-candidate`, target
  `task089-target-host-candidate`.
- Runtime `minicore-runtime-task089`, target `task089-target-runtime`.

All are under `/workspace/shared`. Test sources and outputs are private review
evidence under `/workspace/shared/task089-evidence`. Additional integration
probes are temporarily linked into the two isolated Host test directories;
they are not production edits or deliverable tests. No build artifacts from
other worktrees are reused. The candidate uses an external Cargo config patch
and a local-only lockfile lacking only the patched Runtime Git source entry.
The exact committed lock is restored before handoff. No override, manifest,
lockfile, dependency pin or private probe is committed.

Common test environment:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
# CARGO_TARGET_DIR is selected from the three exclusive directories above.
```

Rust is `1.90.0 (1159e78c4 2025-09-14)`. All Cargo validation is locked and
offline. Loopback HTTP supplies deterministic provider fixtures, not live-provider
results. Host, resources, storage, tools and Runtime remain actual production
implementations wherever the tests describe them as such.

## Review observations

- Control Records are returned through `ConnectionRegistry::accept` and emitted
  after both that guard and the Session mutex have ended. Host control forwarding
  holds no outer Host/Agent mutation guard. Receipt operation IDs and committed
  revisions are retained. The five-scalar scheduling comparison is transient,
  not a log-dedup cache or additional lifecycle state.
- A load has two different committed facts, Loading and its eventual result.
  Both use the initiating load operation. A repeated pause without a changed
  scheduling projection produces no further scheduling record.
- The cancel record is gated on the actual first `Claimed` result. A completed
  old target, a cancelling/finalizing target and a duplicate request cannot
  manufacture another claim or borrow the current Turn's identity.
- New Closed hooks compare the actual before/after lifecycle while locked and
  emit scalar records after unlock. Driver/preparation, Turn finalization,
  handoff, scope cleanup and member-release paths preserve their previous close
  barriers and callback order. Already Closed and unpublished owners are
  excluded. The call graph was traced through Agent close/delete and Host
  shutdown; their child calls do not carry an owner-state lock into emission.
- Model retry logs observe actual Runtime RetryWaiting updates; attempts and
  summaries come from the joined original LoopReport, not a watch estimate.
  `request_index` stays Loop-local. Public `response_index` adds the retained
  original `response_base`, including tool round trips and later Loops within
  the same Turn. Complete response usage and partial usage are selected by the
  original request index, not compacted response order.
- The proposed wait field exists only on Runtime LoopReport. The unique
  ModelDriver samples its monotonic wait interval and includes interrupted
  waits. Host reads the same indexed entry only for `model.recovered` INFO,
  converts to milliseconds and omits unavailable/unrepresentable values. It
  does not infer elapsed time from Retry-After, deadlines, client countdowns or
  Host observation lag. No client event/API expansion is part of this review.
- A model summary does not become a second principal ERROR. The execution
  failure and an independently failed save retain distinct original Faults;
  explicit same-save continuation does not call the model or re-emit its
  report. Record data contains only IDs and declared scalars, never owner Arcs,
  response bodies, requests, credentials or retained callback state.

## Validation and independent probes

The following commands are actual completed runs, not inherited gate results.
Paths below are relative to their fixed worktree. Each `.log` is under the private
evidence directory above. No 900-case workspace rerun was requested or claimed.

### Original pinned Host

```sh
cargo test -p minicore-host-transport --locked --offline \
  --test diagnostic_controls --test diagnostic_close_bridge \
  --test diagnostic_model --test diagnostic_terminal_failures \
  --test diagnostic_calls -- --nocapture
# host-baseline-focused.log: five existing actual integration cases passed.

timeout 90s cargo test -p minicore-host-transport --locked --offline \
  --test task089_private_reentrant -- --nocapture
# reentrant-controls-final.log: one new compound review probe passed.

cargo test -p minicore-host-core --lib --locked --offline close -- --nocapture
# core-close-focused.log: two cases passed, 99 filtered out.
cargo test -p minicore-host-core --lib --locked --offline close -- --list
# core-close-list.log: the same two uniquely named cases.
cargo test -p minicore-host-core --lib --locked --offline \
  task071_result_handoff_and_scope_cleanup_are_published_before_waits -- --nocapture
# core-handoff-focused.log: one case passed, 100 filtered out.

cargo clippy -p minicore-host-core -p minicore-host-transport \
  --lib --locked --offline -- -D warnings
RUSTDOCFLAGS=-Dwarnings cargo doc -p minicore-host-core \
  -p minicore-host-transport --no-deps --locked --offline
cargo fmt --all -- --check
./scripts/check-whitespace.sh
git diff --check
```

The new reentrancy probe uses the real production Host/resources/Runtime and a
scripted loopback provider. It first completes a real Turn with diagnostic
collection disabled. It then exercises pause/repeated pause, unload, both load
transitions, resume, a completed old Turn's cancel while a different Turn is
active and another input is queued, first cancel and duplicate cancel, idle
close and active asynchronous close. Ten diagnostic callbacks independently
re-enter Host info, direct-owner Session state and receipt traversal with a
bounded reply, so an emitting caller that retained the access/Host/Agent/Session
mutex would fail rather than hang. There is one actual cancel claim for the
second Turn, none for stale/duplicate requests, and exactly one Closed event for
each close scenario. No private fixture text enters the captured records.

The existing `diagnostic_calls` case also executes real work before logger
installation and after writer shutdown. This is a disabled/closed-output check,
not a claim of actual disk exhaustion. The other baseline cases independently
retain the real storage-failure and asynchronous close semantics.

### Local candidate and its exact Runtime source

All candidate Cargo commands use the external config at
`/workspace/shared/task089-evidence/cargo-config.toml`, whose only patch points
to the isolated fixed Runtime worktree. `CFG` below denotes this exact path.

```sh
cargo --config "$CFG" test -p minicore-host-transport --locked --offline \
  --test diagnostic_model --test diagnostic_terminal_failures \
  --test diagnostic_tools -- --nocapture
# host-candidate-focused.log: three existing actual integration cases passed.

cargo --config "$CFG" test -p minicore-host-transport --locked --offline \
  --test task089_private_model_indexes --test task089_private_dual_failure \
  -- --nocapture
# model-indexes-dual-failure-final.log: two new review probes passed.

cargo clippy --config "$CFG" -p minicore-host-core \
  -p minicore-host-transport --lib --locked --offline -- -D warnings
RUSTDOCFLAGS=-Dwarnings cargo doc --config "$CFG" \
  -p minicore-host-core -p minicore-host-transport --no-deps --locked --offline
cargo fmt --all -- --check
git diff --check
```

The index probe uses actual production `read` tool execution and seven scripted
HTTP attempts. One Turn crosses two Runtime Loops, with a tool round trip in the
first and an unfinished response with known usage in the second. It verifies:

- Original mappings `(Loop A, request 0) -> response 0`, `(A, 1) -> 1`, and
  `(Loop B, 0) -> 2`, without borrowing another Loop's identity.
- Actual request attempt counts `[2, 3, 2]` and distinct normalized input usage
  `[101, 202, 303]`, including the actual partial at public response 2.
- Exactly two recovery records for the complete retried responses, each with
  source-measured `wait_duration_ms`; the retained run observed `[11, 32]` ms.
  These sample values are not threshold promises or planned-delay estimates.
- No recovery for the unfinished response, no fabricated finish reason, no
  wait duration in `model.finished` or scheduled-retry records, and only one
  principal execution ERROR. Bodies, keys, private tool path and workspace
  path are absent.

The dual-failure probe extends the existing actual HTTP 401 plus real production
history inode-refusal scenario. After restoring the original inode and explicitly
continuing that original save, there is still exactly one `model.finished`
(DEBUG, attempt 1, no complete response), zero `model.recovered`, one primary
execution ERROR and the separate actual history-write ERROR. No model replay or
second summary is introduced by save reconciliation.

### Runtime focused contract run

```sh
cargo test --all-features --locked --offline \
  --test model_retry_state_contract -- --nocapture
```

`runtime-retry-focused.log`: 14 existing deterministic contract cases passed.
These use explicitly fake model/tool adapters with actual Runtime and paused
Tokio time, not a live provider. Coverage includes exact elapsed waits, delayed
repolls, request reset after tools, cancelled and deadline-interrupted waiting,
no observer, non-retryable/normal zero wait, original attempt counts, partials and
unissued preparation. No Runtime production or test file was modified.

### Corrected probe/invocation mistakes, retained separately

Two private probe first runs failed their own overly broad or incorrect
expectations, not the reviewed behavior. `reentrant-controls-first.log` included
ordinary client-call summaries when expecting no scheduling/cancel claim for an
operation; the assertion was restricted to the relevant business events.
`model-indexes-first.log` expected disjoint normalized input/output usage without
providing cache/reasoning details; the provider correctly kept those fields
unknown. The corrected fixture explicitly supplies zero details and places
partial usage on a valid nonempty choice before EOF. The first probe source
versions remain beside the final sources. No production fix was made to obtain
these passes.

A first candidate Clippy invocation put `--config` before the external Clippy
subcommand and stopped immediately with the locked-file-update refusal. Keeping
`--locked` and passing the same external config after `clippy` produced the
successful final run. No dependency version or source was silently changed;
`candidate-clippy.log` retains that invocation failure separately from
`candidate-clippy-final.log`.

## Findings, limits and handoff

No actionable correctness regression was found in the reviewed source ranges.
This is a narrow independent review, not acceptance of every task053 boundary.

- The original Host pin cannot provide elapsed retry wait. Its known SPEC gap
  remains until the consumer is integrated with an explicitly selected Runtime
  revision containing the reviewed report field. This review neither changes a
  pin nor substitutes a local-path gate for the final published-pin gate.
- Compaction's separate locked Closed-completion seam, Agent stop/force logging,
  and other task053 cleanup/handoff diagnostics outside the named source range
  are not newly approved here. The known task087 saved-result restart issue is
  outside scope.
- Asynchronous Closed records keep only their actual Session identity, mode and
  committed revision. They do not invent an original operation or add retained
  lifecycle state; accepted-close/cancel records carry the originating operation.
- Full workspace/all-target lint, all doctests, live-provider behavior,
  Windows/macOS and TUI were not rerun. There is no performance conclusion in
  this review; task090 owns the separate measurement. The short benchmark quiet
  window was honored before resuming the final probes.
- Production files are unchanged. Temporary test links are removed, and the
  candidate's original Cargo.lock is restored byte-for-byte. Only this task
  document is committed. No push, tag, main merge or remote action was made.

## Retained evidence hashes

SHA-256 of the principal logs and captured diagnostic records:

| Evidence file | SHA-256 |
| --- | --- |
| `host-baseline-focused.log` | `c3b34505f6fb16b555548082cb6e1f471eef5e426d733f66d95804464053b351` |
| `host-candidate-focused.log` | `a674d909030cee5fb48c1d0c7e76979c0094da1733c5d628033cc0ebd3dcefc4` |
| `runtime-retry-focused.log` | `f9101de3b0f7222e9f9dfa8282f833664850482a86ded97ee0ce459ef2d18101` |
| `reentrant-controls-final.log` | `ce63c0aa8a3b10ca5a0a016db54c99a5d057c1705c34832adca6ef7faa69b753` |
| `model-indexes-dual-failure-final.log` | `2c6d02a22630486f434dc22118cf0692a2288534bc48c4b94d4c0bb5c3fd39f8` |
| `reentrant-controls-records.json` | `a638fca6dd278fec31054311b95a2c2702f0b64c143769d86c656aaa628f3ff6` |
| `model-indexes-records.jsonl` | `2b2d036e18008d0148d435027034a08a1bfaaaedff4e65935a6c56ba166d54d3` |
| `dual-failure-model-records.jsonl` | `b44fcb3c0c23279882bd2d04b50311fb2e1e853166caf898877ccbf322768a77` |

The evidence directory's `SHA256SUMS` additionally covers the probe sources,
external config, lock copies, remaining focused checks and retained failed
invocations. Evidence files are private review artifacts, not repository source.

Evidence manifest SHA-256:
`c00aac928c6ff66e168333439971016bf9ebd2331674aa6ba87b225697f58a36`.
