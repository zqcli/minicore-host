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
