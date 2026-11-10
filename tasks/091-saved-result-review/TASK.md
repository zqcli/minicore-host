# Task 091: Independent saved Turn result review

## Verdict and fixed sources

**APPROVE the saved-result scope at
`7b6f14080a1fa1d3ac0dbe8623dd59f641dbdadf`**, tree
`868ee620e75a5dbcbdce5ad691675fef217591bd`.
The real restart-result P1 is fixed. This review reproduced one additional P2
in the first candidate's result DTO, then independently verified its narrow
correction from red to green. No unresolved P0/P1 or further blocking finding
was established in this scope. This is not full first-version acceptance.

- Comparison base: `d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06`.
- Task 087 commits: original red `71d393f8c0d99c5c1ce2b4f69cca2f1c0ea67914`,
  implementation `98d476f045e1f775a56696339a65451a7b1f10ed`, documentation
  `6032f8f626a1f73303587cc7a8bb2988a50c341b`.
- Initial independent review/test source: `6032f8f`, tree
  `560580b4572991c06fad6cc6f0892d6e5ea44862`.
- Coordinator correction: `7b6f140`, after integration of task 087. The final
  independent run used that exact complete commit, not an uncommitted patch.
  Concurrent diagnostics changes in its ancestry are not accepted by this review.
- Requested reviewer configuration: `gpt-6-astra`, `max`. This records the
  requested configuration, not independently verified effective runtime settings.
- SPEC v4.93 remains byte-identical, SHA-256
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
  Runtime remains `69faf9196991524b5549499d6208a1eaf567bf56` / 0.10.0;
  manifests and lock were not changed by this review.
- Minicore Agent remains a code reference, not a compatibility baseline. TUI
  remains unchanged/deferred at `3beeb1d8a525e58ce6352e05061f242c5580e248`.

The reviewer read AGENTS, SPEC chapters 2/4/8/9/11 and Rule 20,
[task 087](../087-saved-turn-results/TASK.md), the coordinator's
[current contract audit](../../docs/current-contract-audit.md), the production
delta and relevant owner/storage/codec implementations. The audit's original
saved-result finding is historical input, not an acceptance claim for later code.
The isolated detached worktree is `minicore-host-task091-review`; its target was
never shared with another worktree. The reviewer changed no production source,
main checkout, manifest, lock, SPEC or TUI, performed no remote action, and
committed only this review document. Independent probes remain untracked and
copied to private review evidence.

## Finding closed during review

**F1 / P2: unvalidated retained/saved result combinations.** At `6032f8f`,
`crates/host-core/src/query.rs` made `info`, `config` and `saved_terminal`
independently optional with derived serialization/deserialization. The enclosing
snapshot also had no association check. The RPC client decoded method results
with `serde_json::from_value` and no later validation. Consequently the DTO and
actual stdio client accepted missing terminal metadata, single-sided retained
metadata, mixed retained/saved metadata, retained metadata without its LoadId,
and a saved result with an invented LoadId. Invalid public structs also serialized.

This violates SPEC Rule 20, line 68: required fields and valid combinations must
be constrained by actual types/validation, rather than an all-optional structure.
The LoadId issue also violates its original-residency meaning in chapters 2/9.
The finding is **not** evidence that a real owner emitted a fabricated result.

The independent wire probe used an actual Host/Runtime result as its valid live
baseline, then a deliberately controlled duplex peer to feed malformed replies
through `HostClient::stdio`. It is a client/codec probe, not a production Host
emitting those replies and not the OS-pipe integration test.

- Nine invalid DTO/source shapes were accepted before the fix.
- All 18 malformed `turn.result`/`turn.wait` method replies were accepted before
  the fix. An unrelated subsequent `host.info` still worked.
- The initial four-test probe had three expected failures and one positive
  control pass. Legal retained/saved shapes and an unknown structured Fault
  retained their exact facts in that positive control.
- At `7b6f140`, the **unchanged** four-test probe passes: all nine invalid shapes
  and all 18 malformed replies are rejected; invalid struct serialization fails;
  valid shapes and unknown Fault information are still preserved.

The correction keeps the public fields and two valid JSON shapes, validates both
directions via narrow wire helpers, and checks LoadId at the snapshot boundary.
Serialization borrows bodies rather than cloning full result history. The saved
backend, result projection and wait implementation are unchanged by this patch.
No general query framework, historical metadata reconstruction or broad DTO
rewrite was necessary.

## Reviewed ownership and result behavior

- **Actual saved results:** SPEC lines 364 and 1415 require saved results to remain
  readable. Fresh real Host/Runtime/production-provider loopback HTTP/SSE tests
  now pass through controlled shutdown/reopen, cold and explicit-load reads,
  complete/tool content, failed/cancelled partials, later new execution and
  persistent close. Original Turn identity, actual stored response indexes and
  terminal reason survive; config, LoadId, counters, revisions and full Faults
  absent from storage are not invented. Scripted HTTP is not a live-provider run.
- **Cold versus loaded coordination:** cold `read_saved_turn` validates the whole
  file before filtering. Loaded reads use only the existing journal's validated
  opening baseline, avoiding a second reader against its exclusive writer lease.
  They do not use that baseline as current execution history. The real gated
  load-before-owner-publication test passed without waiting for execution.
- **No query-based save confirmation:** retained completed/active ownership wins
  before and after I/O. Current active/finalizing results cannot fall back to disk.
  The real journal FullWrite fault regression preserved readable complete content
  and the original save failure, refused a premature result, then finished only
  after explicit original-save continuation. Reading never calls load/reconcile,
  appends JSONL, populates completed/output caches, or starts an old tool/model.
- **Cancellation, revocation and release:** the reader selects on original caller
  revocation and rechecks access/current ownership after I/O and projection.
  Gated tests passed for revocation returning before disk completion while a
  separately accepted load continues, real retirement joining in-flight disk I/O,
  and body reclamation preventing stale content. The disk-only mutex is not an
  owner/admission lock. No owner lock is held over disk I/O.
- **Wait ordering:** active waits subscribe before their second state check;
  the watch channel preserves a completion between that check and `changed`.
  A saved/unknown target goes directly to the result path without recreating old
  waiters. Retained identity/revision, timeout/drop isolation and actual
  typed/OS-pipe/authenticated-WSS result/wait equality tests all passed. This is
  focused evidence plus ordering review, not exhaustive scheduling exploration.
- **Unknown and missing data:** valid unfinished history does not imply a terminal
  outcome; corrupt tails and absent files retain their load Faults. An additional
  independent production-backend probe appended schema version 999 after a valid
  terminal record: the read returned 3302 and preserved every byte. Another proved
  that loaded saved reads return the old opening records but not a new append as
  a live cache; after writer release a fresh cold reader sees that actual append.
  These terminal-only records are explicit storage fixtures, not execution claims.
- **Fault semantics:** unknown Fault leaves still retain type/code/message/details
  through the actual stdio codec. Saved terminal reason remains text, not a
  reconstructed structured Fault. Common RPC decoding serves stdio and WSS;
  malformed-reply probing was done on stdio, not independently on WSS.

## Independent commands and outcomes

All Cargo commands used Rust 1.90.0 and:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-task091-target
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
```

Initial-candidate red command, at `6032f8f`:

```sh
cargo test -p minicore-host-transport --locked --offline --test task091_saved_result_wire_review -- --nocapture
```

Result: 1 passed / 3 failed as described above. The reviewer also ran the two
new storage probes and the original focused owner/transport regressions at this
source; they passed. Task 087's older `d2d0c97` restart-red log was rehashed, not
independently rerun on that old source.

Final independent commands, all successful at exact `7b6f140`:

```sh
cargo test -p minicore-host-transport --locked --offline --test task091_saved_result_wire_review -- --nocapture
cargo test -p minicore-host-core --locked --offline --test task091_saved_reader_storage_review --test task087_saved_result_restart -- --nocapture
cargo test -p minicore-host-core --locked --offline --lib query:: -- --nocapture
cargo test -p minicore-host-transport --locked --offline --test task087_saved_result_contract --test task054_query_contracts --test task070_query_recovery -- --nocapture
cargo test -p minicore-host-transport --locked --offline --test adapters timeout_and_dropped_wait_do_not_cancel_or_resend_accepted_work -- --nocapture
cargo test -p minicore-host-transport --locked --offline --test wait_budget -- --nocapture
```

Per-target results: wire probe **4**; storage probe **2**; restart **4**; filtered
core query units **11**; saved-result adapters **1**; task 054 **6**; task 070 **3**;
wait cancellation/isolation **1**; wait-budget/unknown-Fault **6**. These are
focused run results, not a full-workspace inventory or acceptance percentage.

The following independent checks also passed at `7b6f140`; the two Clippy
commands are deliberately scoped, not an all-target/full-workspace lint claim:

```sh
cargo fmt --all -- --check
./scripts/check-whitespace.sh
git diff --check
cargo clippy -p minicore-host-core --lib --test task091_saved_reader_storage_review --locked --offline -- -D warnings
cargo clippy -p minicore-host-transport --test task091_saved_result_wire_review --locked --offline -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core -p minicore-host-transport --no-deps --locked --offline
```

All relative Markdown links in this report were resolved against the worktree
and exist. Final staging was checked to contain only this document.

### Evidence SHA-256

Files are in reviewer-local `/workspace/shared/task091-evidence/`:

| File | SHA-256 |
| --- | --- |
| `initial-wire-probe.log` | `bea2b533f6bca1e398abe8ce2fa5646645383cdf8301bcf2f6ed1c73f8ff67d8` |
| `focused-owner-gate.log` | `acb57c316b03e459984d03fac620037eda7471983bf01aec1e02a88ad48d625e` |
| `storage-probe.log` | `a4e8f5dfc497d78462d31d8a0a2db4d20ed8ebdd9e390a044cc188c7fa2b1a8e` |
| `fixed-independent-green.log` | `ea13ad112423e54d186c1ef139421a6a79615db562a976759cac5d273461b96e` |
| `review-checks.log` | `5f387281f41f9d3bad83099a6fe5f3c300f2933685f99b43ba1446181aa42b0b` |
| `task091_saved_result_wire_review.rs` | `0bd3ea7c15d238454e0dff007381764c8c29e1cd559dbc3d0b17d22ae2d71a38` |
| `task091_saved_reader_storage_review.rs` | `12b91da198c216a628d7077aaf7af224c770f8aa5ef659ff1f98ec2fc977c3d5` |

## Limits and handoff

The producer's affected 480-case gate and logs were inspected/rehashed, not
relabeled as this review's independent full gate. This review did not run the
entire workspace suite/list, full workspace lint/docs, Python performance
analysis, live external models, TUI, crash-durability acceptance or every possible
thread interleaving. The coordinator owns the composed gate and publication.
Later Runtime, diagnostics, performance and other P1 work remain separate, including
`last_compaction`; this review did not expand into their implementation.

No full historical execution API is promised: current-run native owner APIs,
`turn.get`/`turn.list`, original tool/output buffers and collaboration bindings
were not changed into restart recovery. The accepted repair is a narrow read-only
projection of actual saved facts, with valid public result branches.
