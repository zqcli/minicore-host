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

## Follow-up implementation and regression coverage

- Initial implementation commit: `28f1ba0` (`feat(core): retain and continue original history saves`).
- The Runtime may retain `Completed` plus independent `history_failure` for a fully formed final no-tool response; a callback failure before tool execution/next request can instead produce `Failed(History)`. Tests assert the actual report's primary result, never replace either with synthetic success or failure.
- Original driver handles are joined before explicit save continuation, including when `resume` arrives between public finalizing and actual driver return. Synchronous backend future-construction panics are captured as save faults. Runtime input/history boundaries consume the Host force fence before another business step, without discarding a save.
- Deterministic fault fixtures use the production JournalHistory/SessionJournal, real files, actual acceptance ranges, real reconciliation/sync, and actual Runtime. Only write interruption is injected: zero bytes, a prefix, or all original bytes without sync, followed by an explicit error. These are not live filesystem failures or provider/TUI claims.
- Coverage includes original zero/full write, altered/partial blocking, no duplicate append, no tool execution before save, no repeated already-executed tool, no second model request, sequential failures at input/partial/terminal ranges, completed/failed/cancelled outcome preservation, force, abandoned waiter and revoked connection, repeated resume, stale Turn/Load/range rejection, and a fresh later Load.
- New public integration test `tests/history_continuation.rs` incorporates the independent auditor's original-inode restoration regression. It exercises production Host/Journal/Runtime with only a gated model fixture, restores the original inode after a real path-identity append failure, and verifies explicit resume completes the retained range exactly once. It then starts a new Turn/Load and verifies an old Turn target cannot settle/cancel it.
- `cargo test -p minicore-host-core --all-targets --locked`: 76 tests passed (13 unit, 1 history continuation, 1 HTTP, 42 persistent owner, 19 runtime owner). All existing integration files remain unchanged. Hook-related dead-code warnings remain until sibling lifecycle integration; no Clippy success is claimed yet.

- Fault/continuation regression commit: `f179a0d` (`test(core): exercise original save continuation boundaries`). A following focused run passed 15 unit tests and the public continuation integration; the two additions exercised no-Tokio memory reclamation and synchronous future-construction panics.
- Subsequent `cargo test -p minicore-host-core --all-targets --locked` passed 79 tests (16 unit, 1 continuation, 1 HTTP, 42 persistent owner, 19 runtime owner). The additional deterministic worker-tail fixture proves a save/business acknowledgement cannot replace actual task join and root release.
- Resource release now returns `Result<(), Fault>`, not a persistence value, so retirement cannot be mistaken for original-record save acknowledgement. Backend release panics are reported without declaring resources retired.
- Host lifecycle integration dependency `577abca` was cherry-picked only for combined testing. The final small Session hook commit makes its old wait helper test-only and adds post-release memory lifetime reclamation; persistent Session lifecycle/history is not closed or deleted.

## Session-owned metadata continuation and scope

SPEC §11 explicitly allows `resume` to reconcile this run's original saves. The same Session entry now captures its own pending history and/or metadata originals and starts each once. They retain separate range/candidate/baseline confirmation. Scheduling resumes only after both succeed and no later pause won. A retained closure record can finish while Closing, but Closed is never reopened. There is no Agent/unpublished-creation recovery API and repeated Host shutdown is not a retry trigger.

Three additional production Host/Journal/Runtime integration cases passed with `cargo test -p minicore-host-core --test history_continuation --locked` (4/4 total): metadata exact candidate and changed-record rejection; independent simultaneous history/metadata confirmation; original closure completion without reopening. The finalizing Turn remains pending after a non-terminal original range is confirmed, until its remaining partial/terminal handoffs are confirmed.

Per the owner's clarified first-version scope, dropping the final root `Arc<Host>` after saves have failed is treated as uncontrolled instance destruction, not normal stopped retirement. No permanent self-retention cycle or new destruction/recovery architecture is added. Connection revocation, abandoned request/wait futures, and explicit shutdown retain their accepted save owners normally. Unpublished-create reconciliation remains outside this stage; no automatic repair, retry, or artificial confirmation is supplied.

- Metadata resume implementation: `4dfef35` (`feat(core): reconcile Session metadata through resume`).
- Resource-retirement distinction and final Session hooks: `291d36b` (`fix(core): distinguish save confirmation from resource retirement`).
- With sibling Host/memory integration (`577abca`, `4413b74`, `2080485`), `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo fmt --all -- --check` passed. The temporary history-only internal alias is removed after the sibling's stopping route adopted `continue_saves_owned`.
- The final combined-original regression now leaves a real follow-up queued while metadata remains failed. Confirming history alone does not resume it; after the exact metadata candidate confirms, a distinct new Turn starts using the new configuration revision. This tests the resume gate without inventing a new public state, retry layer, or recovery queue.
- Latest focused command `cargo test -p minicore-host-core --test history_continuation --lib --locked` passed 20/20 (16 unit + 4 integration). Combined all-targets also passed 7 entity retirement, 10 Host shutdown, 1 HTTP, and 42 persistent owner cases. Four existing runtime owner assertions still expected readable closed-memory tombstones; the lifecycle owner is updating those public-contract assertions. No full final workspace pass is claimed until that integration update and fresh-target verification.
