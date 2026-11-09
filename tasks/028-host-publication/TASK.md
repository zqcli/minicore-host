# Publish the foundation checkpoint

Status: published and exact-head CI passed. The first v4.93 release is not complete, and no main merge was performed. See AGENTS.md for shared constraints.

## Fixed handoff

- Repository and branch: `zqcli/minicore-host`, `dev`.
- Published head: `82583b31ac9091164da5f6dcfee6bb23e05137e5`.
- Compared with unchanged README-only remote main: `b341b163e14543bfbe7f8d392ea243b4a123cdf7`.
- Frozen source: `797622fa9d663db01fad0d8b800a51a181ddc150`; code checkpoint `275110695551c30582d1302e6b01ab23a3aad536`.
- Both final trees: `c71200b696b861e08ecb29309fdd0ffc7f48a236`.
- [Source-to-published mappings](../../docs/host-publication.json) cover all 101 commits, including source SHAs in previous task records.

## Publication and checks

The approved GitHub interface recreated commits after the remote README ancestor, preserving each tree, raw commit message and parent order. Default author and committer are `zqcli <276883664+zqcli@users.noreply.github.com>` throughout. No message contains a co-author trailer. All 101 raw commit objects, their recursive tree entries and 511 distinct referenced blobs were verified. The full SPEC hash is unchanged. GitHub ref reads and `git ls-remote` independently agreed on both refs. There was no force update, tag, PR or other ref change.

The frozen source passed `./scripts/check.sh` and `cargo test --workspace --all-targets --locked -- --list`: 340 unique tests and 3 doctests, plus formatting, strict Clippy/rustdoc and whitespace. [Hosted run 42629644954](https://github.com/zqcli/minicore-host/actions/runs/42629644954), head `82583b3`, and its quality job `129202312323` completed successfully on 2026-11-09 UTC. Its complete `./scripts/check.sh` step passed. The action emitted a Node 20 deprecation notice and executed using Node 24; this was not a failed check.

## Scope and continuation

The review scope, remaining methods and evidence boundaries are in PROJECT.md, docs/review-handoff.md and docs/method-coverage.md. Runtime is pinned to published `59ee8ce668f72a21d408e6247d3ce075b260db14`; neither later local Host integration work nor the later Runtime retry interface is included in this foundation checkpoint. Agent/TUI references are unchanged and were not jointly integrated. Continue development in isolated topics, retain this checkpoint and require independent review before any main merge.
