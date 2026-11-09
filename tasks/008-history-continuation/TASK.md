# Continue Original History Saves

Status: implementation in progress; integration with Host retirement is pending.

## Scope

- SPEC v4.93 §§3–5. Base: `6d29cdca4c265c3f1eaee906d97f7855c9d74545`.
- Runtime remains the published, locked `18a6ac9b82d003330cd870275efca793cdcc9a96`. No local dependency patch, Runtime modification, storage modification, or TUI work.
- Session owns accepted incremental history and its exact Turn/Load/Loop boundary through failure. Existing `session.resume` explicitly continues the retained original append, then hands off any unseen report suffix, partial, and terminal record exactly once. It never reruns Runtime/model/tools or changes the real primary outcome.
- Production JournalHistory retains both pre-accept records and accepted SessionJournal ranges. Reconciliation invokes the actual original journal, not reopen/readback acknowledgement. Partial, changed, extra, replaced, or uncertain bytes stay blocked. No automatic retry/repair or new canonical RPC is introduced.
- Session resource retirement fences new work, waits real retained worker handles, and releases actual journal/store/root/resource references. Closed memory reclamation clears retained bodies/results and invalidates old observation handles independently of Arc lifetime.
- Shared lifecycle dependency is the sibling's `08025ac` control-fence commit (locally cherry-picked as `b25da9f`). Integrators should include the original once, not both. Host/Agent/lifecycle types remain the sibling's exclusive ownership.

## Verification so far

Environment: Rust/Cargo 1.90.0; `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, `PATH=/workspace/shared/cargo/bin:$PATH`, `TMPDIR=/workspace/shared/minicore-host-tmp`, and independent `CARGO_TARGET_DIR=/workspace/shared/minicore-host-history-target`.

- `cargo fmt --all`: passed.
- `cargo test -p minicore-host-core --lib --locked`: 5/5 existing unit tests passed. Session shutdown hooks are not yet consumed by this isolated snapshot, so dead-code warnings are expected until lifecycle integration. This is not a Clippy or full-suite pass.
- Further real-file, deterministic gated continuation/regression tests and final complete checks remain in progress. No remote push was performed.
