# Integrate controlled shutdown and memory retirement

## Scope and status

Implemented and integrated on `dev`. This stage covers controlled Host shutdown,
Session-owned original history/metadata save continuation, and normal memory
entity retirement. It does not complete v4.93 or replace the requested external
review. TUI, Runtime, provider, storage, manifests and lock remain unchanged by
this lifecycle integration.

## Topic-to-dev commits

| Original topic commit | Integrated commit | Change |
| --- | --- | --- |
| `08025ac` | `f273ec5` | Host admission/execution fences |
| `28f1ba0` | `afc49ae` | Retain and continue original history saves |
| `577abca` | `2734eb9` | Owned graceful/force shutdown |
| `f179a0d` | `754d318` | Save-boundary regressions |
| `291d36b` | `9966dae` | Separate confirmation and actual retirement |
| `4413b74` | `a26dda7` | Retire closed memory owners |
| `4dfef35` | `2a7ffa9` | Session metadata confirmation through resume |
| `2080485` | `2ed6d29` | Stopping resume routing |
| `1a59cbd` | `b9c3858` | Atomic close/watcher exit handoff |
| `ace9a70` | `ab0e589` | Resume after all captured originals confirm |
| `85fc293` | `413916c` | Final lifecycle task scope/evidence |
| `5b2e911` | `3874e9a` | Final Session task evidence |

`7fc6934` updates five inherited runtime-owner tests that expected closed memory
objects to remain readable. They now require actual missing-source Faults after
reclamation while retaining original unload/history, default/no-fallback,
preparation, panic and acceptance-order assertions. The ordinary default-close
test keeps a second child open so Agent retirement cannot obscure its default
selection assertion. Other state-wait helpers were not loosened.

## Verification

Actual toolchain/environment:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
cargo test -p minicore-host-core --test runtime_owners --locked
# Repeat the same 19-test command 25 times; every run passed.
./scripts/check.sh
```

The full gate at fixed `413916cac0730d291e5c8163cdc3d59e5dbedcd1` passed:

- 240 unit/integration tests: core 100, protocol 47, provider 36, storage 57.
- 2 doctests, formatting, all-target workspace Clippy with warnings denied,
  rustdoc with warnings denied, and committed/staged/unstaged whitespace checks.
- Full log: `minicore-host-validation/retirement-integrated-413916c.log` in the
  engineering workspace. The subsequent `3874e9a` changes documentation only.

Core breakdown: 16 unit, 8 memory retirement, 4 original save continuation,
10 shutdown, 1 real HTTP provider chain, 42 persistent owner and 19 runtime owner.
No skipped project tests were used for this integrated gate. Repeated tests are
scheduling regression exposure, not distinct coverage or a capacity benchmark.

A separate read-only review materialized fixed topic code `6914e92` in its own
snapshot and build target. It passed 12 additional public-API acceptance cases,
plus 16 unit/10 shutdown/8 retirement/4 continuation repository cases. It found a
normal concurrent close/watcher-exit handoff window in the earlier code; the
`1a59cbd`/`b9c3858` owner-epoch fix and real concurrent-close regression close it.
The independent report is retained as `minicore-host-audit-artifacts/FINAL_REVIEW.md`.
It is a scoped local review, not the user's separate external review.

Tests use actual Host owners, Runtime, Journal and temporary files; model/tool
fixtures are explicitly in-process. The existing real HTTP/SSE chain also passed.
No live external model, managed subprocess/DSH or TUI validation is claimed.

## Required semantics and explicit limits

- `stopped` follows real joins, required save confirmations and resource release;
  persistent Sessions are not implicitly closed or deleted. Old observers/waits
  and public Host handles cannot retain a successfully stopped root lease.
- Repeated shutdown reports actual current facts. It does not restart work or
  retry failed saves. Session resume confirms its own retained history/metadata
  originals independently; no duplicate append or business rerun is introduced.
- Closed memory sources are removed after cleanup. Ordinary Turn completion,
  cancellation with retained inbox, and a temporary lack of work do not reclaim
  the Session. New creation and last-child retirement share the direct owner.
- The owner deferred unpublished-creation save recovery. The uncommitted storage
  experiment was reverted before integration. There is no new recovery method,
  candidate/witness layer or automatic repair; necessary failure is not success.
- Final root Host destruction after an already-failed save without controlled
  shutdown is uncontrolled instance destruction, not normal stopped retirement.
  No permanent self-cycle was added. Abandoned call futures, revoked Connections
  and accepted explicit shutdown retain normal owners and remain tested.

## Next work and publication

Canonical typed control/receipt mapping and workspace/change reads continue on
separate topic worktrees. Phased persistent deletion, broader configuration and
transport, DSH, tools, observation, interactions and compaction remain open.
Host has not been pushed; no remote publication or main merge is authorized by
this stage. Current local authors are `Codex <codex@openai.com>`, without co-author
trailers, not the `zqcli` author used by the published Runtime API commits.
