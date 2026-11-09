# Recover the development workspace

## Verified recovery boundary

The execution environment was replaced on 2026-11-09. The former local Git repositories, worktrees, build outputs and original test/performance artifacts were absent from the replacement filesystem. This record distinguishes restored Git content from historical reports and new reconstruction.

- Public `dev` remains `1a2935f60ad5cb58b6d45b82f40883149a9b4044`; public `main` remains `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. Both were rechecked through GitHub. Exact-head Host CI 42680053401 remains successful.
- A full, non-shallow clone and official Rust 1.90.0 with rustfmt/clippy have been restored. Git fsck and toolchain version checks pass. No Rust build/test was run by the workspace-restoration task.
- Twelve subsequently created GitHub commit objects are intact even though the dev reference was never advanced. Their actual complete messages, trees, authors and parent chain are in [the prefix map](../../docs/recovery-prefix-map.json). They descend from the published dev; they are not a published branch checkpoint.
- The thirteenth source step's tree `ba1c10a7b15bad746761272d7b87a564dfb29efa` was also read back and verified: 374 files, 4,657,145 bytes. Its prior commit-creation response was not obtained, so that remote commit identity remains uncertain and was not retried.
- New local recovery commit `7cc0bd2b399c3ea1320647e13d87bf57376934a5` retains exactly that tree on confirmed parent `fcab3297cc5e623bda79b81331896ebf182f4f95`. This is a new local identity, not a reconstruction of the uncertain remote commit. Its tree equals the formerly local source `bee13dcfeb601f7e044cb8ff770bcd548ac13dc5`.

The former final documentation checkpoint `7261dfc1c9ed3fe1176a955e329e628cf5511dfe`, code checkpoint `b2915512dda82cb9caec301666f28b52697733d1`, and latest local probe/documentation head `aa11b4ff7045ba49698faa725f2205763782a040` have not been recovered as complete Git objects or file trees. Do not report them as available checkouts.

## Content and validation status

The restored prefix preserves input provenance, original bound Turn configuration, public query projections and their then-existing tests. Later terminal revision corrections, owner summaries/pagination, complete business diagnostics, the default Turn-inspect payload correction and later measurement documentation are outside this exact prefix. Available verbatim patches/files are being recovered separately; missing source will be explicitly reimplemented and independently revalidated rather than presented as byte-identical recovery.

The historical b291551 full-gate result was 865 unique Rust cases, 4 doctests and 20 Python helper cases. That is not a new gate on this restored or reconstructed code. Original full raw evidence and performance traces were not recovered. Selected historical terminal output/reports are labelled accordingly; new results must identify their own source and files.

Configured credential inheritance remains an open defect. Synthetic-only probes established that actual bash output carries configured model and Host access-key values into the next model request and saved/reopened history. The original production fix had not been written when the environment changed. Two actual-chain regression sources and their baseline-red output were recoverable; no fixed-code green or security review existed. Tasks 066/067 resume on the restored base with initial and cross-reload tests, including already-bound and later Turns, source rename/removal, actual output and durable history. No real credential is used. Filtering inherited names is not same-user process or filesystem isolation.

## Commands and evidence

- Full clone of `https://github.com/zqcli/minicore-host.git`; non-shallow check; `git fsck --full`; `rustc --version`, `cargo --version`, `rustup component list --installed`.
- Read-only fetch of confirmed commit `fcab3297cc5e623bda79b81331896ebf182f4f95` and verified tree `ba1c10a7b15bad746761272d7b87a564dfb29efa` into a separate recovery object store. All recovered blobs and file modes were checked.
- Fetch confirmed history into the checkout; create isolated `stage/recovery-dev-20261109`; apply the exact source-12-to-13 diff; verify `git write-tree` equals ba1c10a7 before the new local commit.
- SPEC remains SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`. Runtime remains published `69faf9196991524b5549499d6208a1eaf567bf56`, whose CI 42624211012 was rechecked successful.
- External restoration details: `/workspace/shared/recovery-workspace/public-repository-and-toolchain.md`, `/workspace/shared/recovery-publication/`, and `/workspace/shared/recovery-root/STATUS.json`. These are restoration evidence, not replacement full test logs.

## Next steps

1. Recover provable component sources and explicitly restore the missing query, discovery and diagnostics behavior with incremental local commits. Shared files have one owner at a time.
2. Complete configured credential exclusion and deterministic real-chain regressions, then obtain independent review.
3. Run a fresh complete gate and new fixed-source performance checks. Retain the old reported measurements as historical, without fabricating lost raw inputs.
4. Publish only a verified dev fast-forward with accurate recovery/publication maps and exact-head CI. Do not modify main or overwrite published history. External final project review and the intentionally deferred TUI/live-provider checks remain separate.
