# Recover the development workspace

## Current reconstructed checkpoint and backup

Public `dev` was advanced normally in two five-commit batches to
`bb650a5232ee701d8e67deced0beb43c54335988`, then
`51e1ee495c82bc446ae4b569726d076880b0c320`. The latter has the same complete
tree `3d0377503725fdfba3d738ec286b749940699ef7` as local
`c872e1d4d039684fa124d8b5a28fab8efc4ab11c`. The existing history, including
the 12 previously prepared Git objects, was preserved; the ten new commits
retain each source tree, raw message and order. Authors/committers were verified
as zqcli with the connected account's noreply address and no coauthor trailers.
[The publication map](../../docs/recovery-publication.json) is the original
verified publication record. Its in-progress CI and pending review statements
are observations from its recorded time, superseded by the later facts below.
Main remains `b341b163e14543bfbe7f8d392ea243b4a123cdf7`; there was no force update.

Automatic [CI 42714687462](https://github.com/zqcli/minicore-host/actions/runs/42714687462)
and [CI 42715345587](https://github.com/zqcli/minicore-host/actions/runs/42715345587)
both completed with failure. Failure details are being checked rather than
inferred from local results. No CI setting was changed or run suppressed.
Backup publication does not assert a completed stage or passing review.

Nine further local commits end at
`c92e27bf09ac1a50a1a88b04dedc5acb9bf798e0`. Their source changes are:

- Task 067: actual binary/WSS/HTTP/Runtime/bash synthetic credential exclusion
  regression, including next model input, native queries, JSONL and reopen.
- Task 070: four existing binary-fixture query paths migrated to the real
  envelopes without reducing behavioral assertions.
- Task 071: five commits for lightweight discovery/pagination, real owner-work
  summaries, end reason, actual adapter tests and no unrequested config clones.
- Task 053: reconstructed provider diagnostics with 44 real-fixture/unit cases
  and one doctest; core/transport business logging is not yet complete.

These are newly reconstructed changes and validation, not byte-identical
restoration of the missing final checkpoint. Fixed dependencies, SPEC and TUI
are unchanged. The five-commit backup cadence does not replace scoped testing,
the later complete gate, independent review or the requested external review.

## Fresh validation and remaining failures

- Credential review at `77b00825e045321d4add70668d3cb208d4dcfc68`
  (the dependency-bootstrap source plus the actual app regression) approved the
  066/067 scope: 419 unique affected-crate cases and the real app case passed,
  with independent baseline-red/fixed-green replay, strict lint/docs and fmt.
  The production fix is already in the published source; the app test is among
  the next nine commits. Only configured names are cumulatively excluded from
  future child environments. Existing child environments and same-user process
  or filesystem access are not isolated. No real credentials were used.
- Query review at `4daeb72d60ce76cc084b9f26134a1d62802e04ea` approved
  the reconstructed terminal revision/snapshot, inspect payload and fixture
  migration scope. All four independently reproduced baseline defects passed
  on the fixed source; five task054 cases, 98 core unit cases and 17 adjacent
  contracts passed. This was not a whole-workspace approval.
- The coordinator's later `./scripts/check.sh` at that same 4daeb72 source
  exited 101 in
  `raw_result_and_wait_keep_complete_and_partial_indexes_across_internal_loops`:
  final server join received OS error 32 (BrokenPipe). Four other cases in that
  target passed. Task 070 is reproducing the real pipe ownership/close ordering;
  no production error suppression, sleep, retry or weaker assertion is accepted
  as a fix. The complete gate is not reported as passing.
- Task 071's source `74350cf514de8b8e8ff182050f203cfea3232bce` passed
  all 11 new cases, the affected core/tools/transport suite, strict Clippy,
  warning-denied docs, fmt and its transport doctest. One earlier broader run
  encountered the same untouched task054 teardown failure. Independent owner
  discovery review and combined validation remain pending.
- Task 053 source `037b24ab3e5af58b68ea6282656281aa9609f0d0`, integrated
  as c92e27b, passed 44 provider cases, one doctest, strict Clippy/rustdoc, fmt and
  locked metadata. This is the provider slice, not complete business logging.
- `cargo fmt --all -- --check` and committed/worktree whitespace checks pass
  on c92e27b. No full gate or new performance run is claimed for that source.

Fresh evidence remains in the workspace under
`minicore-host-recovery-validation`, `task068-credential-exclusion-review-recovered`,
`task072-query-review-recovered` and the component task evidence directories.
These fresh logs are separate from lost historical logs. No TUI, live provider,
capacity or external final-review acceptance is claimed.

## Original verified recovery boundary

The execution environment was replaced on 2026-11-09. The former local Git repositories, worktrees, build outputs and original test/performance artifacts were absent from the replacement filesystem. This record distinguishes restored Git content from historical reports and new reconstruction.

- Public `dev` remains `1a2935f60ad5cb58b6d45b82f40883149a9b4044`; public `main` remains `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. Both were rechecked through GitHub. Exact-head Host CI 42680053401 remains successful.
- A full, non-shallow clone and official Rust 1.90.0 with rustfmt/clippy have been restored. Git fsck and toolchain version checks pass. No Rust build/test was run by the workspace-restoration task.
- Twelve subsequently created GitHub commit objects are intact even though the dev reference was never advanced. Their actual complete messages, trees, authors and parent chain are in [the prefix map](../../docs/recovery-prefix-map.json). They descend from the published dev; they are not a published branch checkpoint.
- The thirteenth source step's tree `ba1c10a7b15bad746761272d7b87a564dfb29efa` was also read back and verified: 374 files, 4,657,145 bytes. Its prior commit-creation response was not obtained, so that remote commit identity remains uncertain and was not retried.
- New local recovery commit `7cc0bd2b399c3ea1320647e13d87bf57376934a5` retains exactly that tree on confirmed parent `fcab3297cc5e623bda79b81331896ebf182f4f95`. This is a new local identity, not a reconstruction of the uncertain remote commit. Its tree equals the formerly local source `bee13dcfeb601f7e044cb8ff770bcd548ac13dc5`.

The former final documentation checkpoint `7261dfc1c9ed3fe1176a955e329e628cf5511dfe`, code checkpoint `b2915512dda82cb9caec301666f28b52697733d1`, and latest local probe/documentation head `aa11b4ff7045ba49698faa725f2205763782a040` have not been recovered as complete Git objects or file trees. Do not report them as available checkouts.

## Original recovery-time content and validation status

The restored prefix preserves input provenance, original bound Turn configuration, public query projections and their then-existing tests. Later terminal revision corrections, owner summaries/pagination, complete business diagnostics, the default Turn-inspect payload correction and later measurement documentation are outside this exact prefix. Available verbatim patches/files are being recovered separately; missing source will be explicitly reimplemented and independently revalidated rather than presented as byte-identical recovery.

The historical b291551 full-gate result was 865 unique Rust cases, 4 doctests and 20 Python helper cases. That is not a new gate on this restored or reconstructed code. Original full raw evidence and performance traces were not recovered. Selected historical terminal output/reports are labelled accordingly; new results must identify their own source and files.

Configured credential inheritance remains an open defect. Synthetic-only probes established that actual bash output carries configured model and Host access-key values into the next model request and saved/reopened history. The original production fix had not been written when the environment changed. Two actual-chain regression sources and their baseline-red output were recoverable; no fixed-code green or security review existed. Tasks 066/067 resume on the restored base with initial and cross-reload tests, including already-bound and later Turns, source rename/removal, actual output and durable history. No real credential is used. Filtering inherited names is not same-user process or filesystem isolation.

## Commands and evidence

- Full clone of `https://github.com/zqcli/minicore-host.git`; non-shallow check; `git fsck --full`; `rustc --version`, `cargo --version`, `rustup component list --installed`.
- Read-only fetch of confirmed commit `fcab3297cc5e623bda79b81331896ebf182f4f95` and verified tree `ba1c10a7b15bad746761272d7b87a564dfb29efa` into a separate recovery object store. All recovered blobs and file modes were checked.
- Fetch confirmed history into the checkout; create isolated `stage/recovery-dev-20261109`; apply the exact source-12-to-13 diff; verify `git write-tree` equals ba1c10a7 before the new local commit.
- SPEC remains SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`. Runtime remains published `69faf9196991524b5549499d6208a1eaf567bf56`, whose CI 42624211012 was rechecked successful.
- External restoration details: `/workspace/shared/recovery-workspace/public-repository-and-toolchain.md`, `/workspace/shared/recovery-publication/`, and `/workspace/shared/recovery-root/STATUS.json`. These are restoration evidence, not replacement full test logs.

## Recovery plan at the initial checkpoint

1. Recover provable component sources and explicitly restore the missing query, discovery and diagnostics behavior with incremental local commits. Shared files have one owner at a time.
2. Complete configured credential exclusion and deterministic real-chain regressions, then obtain independent review.
3. Run a fresh complete gate and new fixed-source performance checks. Retain the old reported measurements as historical, without fabricating lost raw inputs.
4. Publish only a verified dev fast-forward with accurate recovery/publication maps and exact-head CI. Do not modify main or overwrite published history. External final project review and the intentionally deferred TUI/live-provider checks remain separate.
