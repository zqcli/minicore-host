# Check committed whitespace

Status: fix implemented with isolated Git regression checks; aggregate verification follows this commit. This is a quality-gate correction, not full foundation review approval.

## Reproduction

At Host `6d29cdca4c265c3f1eaee906d97f7855c9d74545`, `git diff --check main..HEAD` exits 2 with new blank lines at EOF in root `Cargo.toml:9` and `crates/host-protocol/Cargo.toml:16`. The old `scripts/check.sh` only checked the current unstaged diff, so a clean checkout hid committed whitespace defects.

## Correction

- Remove the two extra trailing blank lines, without changing manifest values.
- Run `scripts/check-whitespace.sh` before expensive Rust checks. It checks committed development from the repository's single root baseline to HEAD, then staged and unstaged changes. It does not silently treat a shallow visible boundary as the original baseline, and rejects shallow or ambiguous multi-root history with an explicit explanation.
- The immutable original project baseline avoids hard-coding local commit IDs that may differ after an explicitly approved publication method. The complete SPEC file remains unchanged.
- CI now checks out full history (`fetch-depth: 0`), retaining the same immutable official action and `persist-credentials: false`. Default ordinary Git clones already contain this history.

## Local evidence

An isolated Git repository exercised five cases: bad committed EOF whitespace is rejected; a later committed correction passes; staged whitespace is rejected; unstaged whitespace is rejected; an actual shallow `file://` clone is explicitly rejected. `bash -n scripts/check.sh scripts/check-whitespace.sh` and `git diff --check main` passed after the edits. Report: `minicore-host-validation/whitespace-gate-regression.json`.

The same independent review also found the separate HTTP 200 SSE Retry-After defect. That belongs to task 010 and is not fixed or hidden by this quality-gate change. A prior test-suite pass is not full SPEC or external review acceptance. No remote action was performed.
