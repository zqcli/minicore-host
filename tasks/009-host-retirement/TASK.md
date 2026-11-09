# Implement Host shutdown and memory retirement

## Scope and outcome

Baseline: `6d29cdca4c265c3f1eaee906d97f7855c9d74545`, complete SPEC v4.93.
Runtime is still the published exact `18a6ac9b82d003330cd870275efca793cdcc9a96`.
TUI, protocol method inventory and persistent physical deletion are unchanged.
No remote writes were made.

Implemented the local direct-owner lifecycle stage:

- `Host::shutdown` uses the canonical request/receipt and current operator. It
  returns current facts immediately, preserves each operation ID, and only upgrades
  graceful to explicit force. Repeated requests do not restart or retry work.
- Host admission fences reject new root input/objects during draining. Accepted
  work drains normally; paused backlog is not silently resumed. Force fences
  consumption and new Runtime work, cancels through actual owners and joins real
  reports, partials, ordinary ToolResults, original saves and cleanup.
- Necessary save failure remains queryable with stopping and the directory leased.
  `session.resume` can continue that Session's captured history/metadata saves while
  stopping, without rerunning business work or reopening a closed Session.
- Host/Agent/Session release real stores, backend handles and execution resources
  only after their obligations settle. A retained Host handle, subscription or
  abandoned Turn wait does not keep the directory locked after stopped. Multiple
  Hosts remain independent, and shutdown never exits the shared process/runtime.
- Ordinary memory Sessions survive multiple Turns and failed/cancelled work with
  retained inbox. Explicit close reclaims them only after actual cleanup. Accepted
  Session creation, last-child removal and empty Agent retirement share the Agent
  boundary. Creating Agents are not prematurely reclaimed.
- Persistent Agents remain without replacement defaults. Shutdown does not mark
  persistent Sessions closed or remove their files. Memory bodies and registrations
  end with their actual ownership lifetime, including completed Host shutdown.
- Existing observers/waits cannot preserve source bodies. Reclaimed sources return
  registered missing-source Faults rather than empty success or cached tombstones.
- Close cleanup has a bounded watcher only while needed. Its exit and a concurrent
  new close share a Host generation boundary, preventing lost cleanup handoffs.
  Shutdown joins any cleanup already owned by that watcher.

## Original commit integration order

Integrate each original commit once; worktree cherry-pick copies are not additional
changes:

1. `08025ac` — Host admission and execution fences.
2. `28f1ba0` — Session original history-save ownership (task 008).
3. `577abca` — graceful/force Host shutdown and initial acceptance tests.
4. `f179a0d` — Session save/retirement tests and immediate-memory hook (task 008).
5. `291d36b` — Session resource retirement and Host-end memory reclaim (task 008).
6. `4413b74` — closed memory Session/Agent retirement and required owner joins.
7. `4dfef35` — Session metadata continuation through resume (task 008).
8. `2080485` — route stopping resume through both original Session save types.
9. `1a59cbd` — atomic close/cleanup-watcher handoff and concurrent-close regression.
10. `ace9a70` — gate resumed work on all captured Session saves (task 008).

Task 008 owns Session/backend changes. This task owns Host, Agent, admission,
core projections, `host_shutdown.rs` and `entity_retirement.rs`.

## Verification

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-retirement-target
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --offline
cargo test -p minicore-host-core --offline --test host_shutdown --test entity_retirement --test history_continuation
cargo test -p minicore-host-core --offline --test persistent_owners
```

Focused tests pass: 10 shutdown, 8 memory retirement and 4 original-save public
integration cases. All 42 inherited persistent-owner tests pass, including immediate
root reopening after awaited successful creation/metadata. Strict workspace gates
and the final filtered workspace test result are recorded below after execution.

Tests use production Runtime and storage with real temporary files; model/tool
fixtures are explicitly deterministic in-process implementations. Coordination uses
actual notifications, Semaphores or held I/O-worker gates, not arbitrary sleeps.
Coverage includes real partial and cancelled ordinary ToolResult persistence,
held-save directory ownership, dropped call/connection/Host handles after accepted
shutdown, paused draining and explicit force, two Hosts, accepted create/force,
close/create competition, concurrent cleanup handoffs, weak observers and missing
sources. This is not live-provider, managed-tool-process, DSH or TUI validation.

The parent owns legacy `runtime_owners.rs` expectation updates. Four tests failed
because they explicitly expected closed memory bodies/Agents to remain queryable;
one additional preparation-close test has the same timing-sensitive expectation.
Their pre-close execution and ordering checks must remain intact, while completed
reclaim requires missing-source Faults. A filtered workspace pass is not an
unqualified full-suite pass; the parent must rerun the complete integrated suite
once those baseline assertions are updated.

## Deliberate limits and first-version scope

- No unpublished-creation recovery API, retained recovery-candidate framework,
  extra storage witness or repeated-shutdown retry was introduced. Draft work in
  that direction was removed before committing, following the user's scope decision.
  A genuine creation save failure can remain stopping/faulted with its directory
  leased rather than claim success. New recovery behavior is outside this stage.
- Losing the last root `Arc<Host>` after work has already failed is uncontrolled
  instance destruction, not a reported stopped state. No permanent self-cycle was
  added for that case. Accepted running work, abandoned call waits, closed client
  connections and explicitly accepted shutdown retain their actual owners.
- Root delegation scopes and the DSH agent-tool surface remain separate foundation
  work. No delegation result/notification is invented to fill that absent surface.
- Persistent deletion, unrelated history, workspaces and independent artifacts are
  not implicitly removed. Full-version transport/TUI acceptance is not claimed here.

Final local verification at `6914e92` (local copy of original `ace9a70`, after
`1a59cbd`): workspace formatting, strict all-target Clippy and strict rustdoc passed.
The following workspace test invocation also passed, including both doctests,
with exactly the five parent-owned legacy expectations filtered out:

```sh
cargo test --workspace --offline -- \
  --skip close_clears_default_and_unload_preserves_memory_history \
  --skip review_memory_default_close_orders_remain_authoritative \
  --skip review_memory_default_close_race_preserves_actual_acceptance_order \
  --skip review_preparation_panic_must_be_reported_and_close_settle \
  --skip review_close_during_preparation_must_finish_after_resolution
```

Focused totals at this checkpoint are unchanged: 10 shutdown, 8 retirement, 4
history/metadata continuation and 42 persistent-owner cases passed. Independent
review at `4413b74` also passed its 11 applicable acceptance cases. Full unfiltered
integration belongs to the parent after its legacy assertion updates; it was not
claimed by this worktree.
