# Implement Host shutdown and memory retirement

## Scope and ownership

Baseline: `6d29cdca4c265c3f1eaee906d97f7855c9d74545`, SPEC v4.93.
Runtime remains the published exact revision `18a6ac9b82d003330cd870275efca793cdcc9a96`.
This task owns Host, Agent, connection admission, public core projections, and new
shutdown/retirement tests. Session/history implementation is coordinated with
`tasks/008-history-continuation/TASK.md`. TUI is unchanged. No remote writes.

## Local commits and checkpoints

- `08025ac`: Host admission/execution fences. Standalone `rustfmt` passed; this
  narrow dependency intentionally precedes its call sites, so no standalone
  warning-free Clippy claim is made.
- Session dependency `28f1ba0` was locally cherry-picked as `397d7a5`; the parent
  integrates the original Session commit once, not both copies.
- Current shutdown implementation: canonical immediate current-fact receipts,
  monotonic graceful/force, owned shutdown task, direct-owner stopping and release,
  releasable Host/Agent store and resource slots, genuine Session task/resource
  retirement, root lease held through pending saves, no implicit persistent close.
  Host waits now hold weak Session observations rather than a Session Arc.

## Checks

Environment for these commands:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-retirement-target
cargo check -p minicore-host-core --offline
cargo test -p minicore-host-core --test host_shutdown --offline -- --nocapture
```

Both passed; 9 shutdown tests pass. Transitional dead-code warnings are the old
Session wait helper (sibling removal pending) and the normal memory retirement
hook (used by the next task commit). Tests use production Runtime, storage, and
real temporary files. Model responses are explicitly named deterministic
fixtures, not live provider or TUI validation. Gates use Semaphores, notifications,
current-thread execution, or a held blocking pool, not arbitrary sleeps.

Covered: paused backlog remains draining; explicit force upgrade and no downgrade;
active accepted follow-ups drain; root inputs/creation reject after fence; genuine
Runtime partial and final history; failed original history continues only through
explicit resume while stopping; dropped waiter/operator/Host handles; necessary
save/root guard barrier; two Hosts; retained observer/wait future; read-only and
revoked operator rejection; repeated and old operation identifiers; persistent
reopen without implicit close; execution resource graph release.

## Remaining work and boundaries

- Add ordinary memory Session removal only after explicit close and cleanup, then
  atomic empty memory Agent retirement; verify create-versus-retire and observers.
- Retain failed initial creation save candidates and support safe original-range
  continuation through later explicit shutdown. No claim that a failed write is
  absent; current conservative barrier keeps stopping and directory leased.
- Root delegation scope and DSH tool surface are not implemented in the foundation.
  No fake delegation, notification, subprocess, or live-provider result is claimed.
- Physical persistent deletion is separate and is not introduced here.
- Full workspace formatting, Clippy, tests and docs gates remain for the combined
  candidate. Existing close-query tests may need expectations changed only where
  reclaimed memory sources now correctly return registered missing-source Faults.

## Memory retirement increment

Shutdown implementation commit: `577abca`.
Session follow-ups: original `f179a0d` / local `72657d9`, original `291d36b` /
local `07d4fe5`. Integrate only one copy of each.

- Ordinary memory Sessions now release after explicit close and actual worker
  joins/resource cleanup. Built-in idle memory can reclaim without a Tokio executor.
  An ordinary final or cancelled Turn with retained inbox does not reclaim it.
- Last child removal, new creation registration and retiring claim use AgentState.
  Creating count protects accepted in-flight Session construction. Empty memory
  Agents retire only after committed creation and settled direct work; Host then
  removes their registration. Persistent Agents remain without replacement defaults.
- Host lifetime completion drops runtime memory registrations and bodies after
  real resource retirement. Persistent Sessions retain their original open/closed
  business state; no file deletion is performed.
- Existing subscriptions contain weak Session owners, and Host waits reacquire
  their source. Reclaimed queries/observers return registered missing-source Faults.
- The cleanup monitor starts only for explicit close work, holds no idle ownership,
  exits when cleanup is done, and is joined by shutdown. Starting it on every
  creation was rejected after a real immediate-root-reopen regression; the existing
  `awaited_creation_retires_its_owner_before_immediate_root_reopen` now passes.
- Added a real Runtime ordinary-tool cancellation/save test, distinct from a
  managed subprocess or DSH delegation integration claim.

Checks on this increment:

```sh
cargo test -p minicore-host-core --offline --test host_shutdown --test entity_retirement
cargo clippy -p minicore-host-core --all-targets --offline -- -D warnings
cargo test -p minicore-host-core --offline --test persistent_owners awaited_creation_retires_its_owner_before_immediate_root_reopen
```

Pass: 10 shutdown tests, 7 retirement tests, strict all-target core Clippy, and the
immediate-reopen regression. Full core run reached 41/42 legacy persistent tests
before the monitor regression was corrected; rerun of that failed case passes.
Legacy runtime tests: 15/19 pass. Four explicitly expect closed memory bodies or
Agents to remain queryable: `close_clears_default_and_unload_preserves_memory_history`,
`review_memory_default_close_orders_remain_authoritative`,
`review_memory_default_close_race_preserves_actual_acceptance_order`, and
`review_preparation_panic_must_be_reported_and_close_settle`. Parent owns those test
updates; their replacement assertions must retain the original pre-close checks
and require actual missing-source Faults after reclaim.

Failed creation follow-up is still pending. Repeated shutdown requests must remain
current-fact controls, not retry triggers. The earlier contemplated interpretation
of repeated shutdown as original-save continuation was rejected before any such
code was written. Last-public-Host-drop handling of already-failed saves is a
separate destruction-versus-abandonment contract question being resolved by parent;
no permanent self-retention was added in this increment.
