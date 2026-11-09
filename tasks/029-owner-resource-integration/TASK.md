# Integrate owners, observations and resources

Status: local integration gate passed, independent scoped reviews and normal-path corrections continue. Not a complete v4.93 release or a newly published Host checkpoint. Shared instructions are in AGENTS.md.

## Fixed boundary

- Integration branch: `stage/observation-delegation`, to be integrated linearly into `dev`.
- Code checkpoint: `d107462e1e67b2270c268a0b4f51bb4ba01bde00`.
- Compare with local foundation `797622fa9d663db01fad0d8b800a51a181ddc150`, published as `82583b31ac9091164da5f6dcfee6bb23e05137e5`. Later documentation-only publication is separate.
- All six Runtime dependency declarations: exact `0.10.0`, Git `69faf9196991524b5549499d6208a1eaf567bf56`. Sole lockfile, no override. Its exact-head CI run 42624211012 passed all seven jobs.
- SPEC unchanged; Agent/TUI references and their older Runtime lock remain as listed in PROJECT.md. TUI is not modified or tested.

## Integrated scope and decisions

Tasks 018–025 integrate scoped event/output/history observation, actual retry/interaction state, direct-owner delegation capabilities and the six-action agent tool, durable title/name updates, authoritative partial reads, production resource configuration and old incomplete-tool-history request projection. Tasks retain source commits and command evidence.

The source owner remains authoritative for acceptance, execution, persistence, result transfer and reclamation. Delegation results are handed off before temporary memory release, not fetched after the fact. Actual Runtime report partials are retained independently of observation. Host configuration has one storage shape; private credentials are prepared outside locks and never serialized in public configuration. Existing Turns hold actual frozen resources. Incomplete old tool calls are represented as historical facts only in the current prompt, without modifying JSONL or executing them. `c2ed00e` connects that helper to production BoundPrompt.

## Corrections and evidence

- Partial-history read defect: source f67e981 integrated as 349f288; the original independent probe passed unchanged on 713efb0.
- Delegation acceptance/load defects: sources 9e6f96d/7c1facb/aa76ff9 integrated as fdbc0d1/63951db/6f343e4. Independent aa76ff9 review passes 450 tests, 3 doctests and seven additional real probes, with both P1 findings closed. The original close probe adjusts only its incorrect assumption that a correctly reclaimed source must still be queryable.
- Deadline interaction label: source 46bd4c9 integrated as 9585b5a. Actual native Deadline maps to expired while the Turn remains cancelled. Independent correction review continues.
- Pipe test teardown: source 583eac9 integrated as d107462. Real unsubscribe response can encounter BrokenPipe when the test closes its reader first. The persistent-history test now half-closes requests and retains responses until server exit; deterministic gates preserve both valid orders and accepted owner execution. No production I/O error is swallowed.
- Scope-cleanup test observed after releasing its gate and raced correct reclamation at 713efb0; 8b2b744 registers the watch first, preserving all cleanup assertions.
- Resource/prompt integration at c2ed00e failed all-target compilation because the old test fixture serialized HostSettings. 9ace676 uses canonical HostConfig; 23be155 fixes its formatting. Neither production format compatibility nor a second schema was introduced. The original failures remain recorded rather than counted as passes.

## Actual final commands

Linux x86_64, Rust 1.90.0; workspace-local Rust/Cargo homes, independent integration target, two build jobs, debug/incremental artifacts disabled. At the fixed code checkpoint:

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
cargo metadata --locked --offline --format-version 1
sha256sum SPEC.md
```

The complete gate passed whitespace, formatting, strict all-target Clippy, 477 uniquely listed target/test-name cases, 3 doctests and strict rustdoc. These are actual owners/Runtime, files, pipes and local HTTP/SSE/TLS fixtures, not live provider/TUI/process or performance acceptance. Logs are retained in the integration validation directory as `stage-pipe-resource-full-published-runtime.log`, `stage-d107462-test-list.txt`, and `stage-d107462-metadata.json`; earlier failures are `stage-c2ed00e-full-published-runtime.log` and `stage-7deba95-full-published-runtime.log`. The commands are reproducible from the source without those local logs.

## Remaining work

Resource/prompt independent review and interaction correction recheck are in progress. Explicit resume/config repair of resource-start faults and the three resource query routes are independent active tasks. Canonical host.reload still needs its owner-held operation receipt, not a transport cache. Compaction/context, ordinary filesystem tools and write observation, workspace routing, remaining canonical methods/deletion, full input source context and real process startup remain unfinished. Enabled compaction is explicitly rejected in production and must be disabled in current fixtures; it is not silently accepted. Preserve the fixed foundation handoff, then publish only reviewed and verified later checkpoints.

## Resource correction and query checkpoint

Code checkpoint `04ba5bc07a7d9bb73eabb143839fbaeb264a0cea` adds source `4f9739d` as `746af80` (13 resource-repair tests), with task026 validation, and sources `cb2b7ec`/`a728177` as `63fbd4b`/`1ece526` (three real shared resource routes, four new tests), with task027 validation. It clears only Missing/Invalid/Conflict startup faults through explicit resume or confirmed config updates; original queued input and explicit pause remain owned, and history/write faults are not cleared.

The same aggregate commands pass 494 unique tests and 3 doctests. Logs are `stage-04ba5bc-full-published-runtime.log` and `stage-04ba5bc-test-list.txt`. Independent review reproduced the full 494/3 gate and 16 real production/repair/transport probes, with no new P0/P1/P2. Observation follow-up independently passed its 477/3 gate, unchanged original deadline/partial/unsubscribe probes and 12 repetitions of the real pipe/history cases. These close the identified scoped findings; canonical reload, compaction, local tools and broader first-version work remain open.

The foundation documentation-only update is now published at `7900d21a8cc767f508d45a1687ade4376bc6dded` and exact-head CI 42632632687 passed. It did not include any of this later integration. Latest engineering instructions require new subjects in `type(scope): summary` form. The 20 older published subjects lacking a scope remain immutable; unsubmitted topic subjects will be normalized during linear dev integration, preserving their trees and granular history.

## Linear dev integration

All 55 unsubmitted stage commits were replayed after existing local dev `f03aaac58a784ffb754007e1d599a9e97e66c5a5`. Two already-published documentation commits were not duplicated. Eight unpublished subjects gained the required scope; files and granular commit ordering were retained. Integrated head `6b22b4d894ee7b82a6b7af1f68f2bc77e742ad66` has exactly the same tree `ff5e06a80a44b5f3c529a8f351fd649e9d8d6e41` as reviewed-stage documentation head `8164ef35826dccdd405d143301e9486e3c57e692`. The [full mapping](../../docs/integration-source-map.json) identifies all source and integrated SHAs. No published Git history was rewritten. Final aggregate dev validation follows on this unchanged code before publication.

## Final dev rerun blocked

At `34641c5ab1784237cb73bdb0f36f9923697d0c34`, `./scripts/check.sh` failed two task024 setup cases: `cancellation_after_old_history_projection_keeps_real_partial_and_original_file_prefix` at seed `read_history` returned HistoryInUse after the journal was dropped; `unsupported_replay_and_signed_reasoning_are_rejected_before_http_not_flattened` failed the following Host open with RootInUse. Both failures occur before model execution. The full original log is `dev-reviewed-final-full.log`; this run is failed, not subsumed by the earlier identical-tree 494/3 passes. Publication is paused for independent diagnosis. A separate pure-std immediate File-lock drop/reopen probe ran 20,000 iterations with no file or directory lock failures; this does not establish the failing path's cause. No production lock change or retry was made on that assumption.

## Lock diagnosis and integrated correction

Task 034 subsequently captured the real sha256sum child retaining each failing inode's flock after the parent closed its descriptor. The test hash helper, not production locking, is corrected. Source bootstrap `0fb766e` maps to dev `ebf59d825b4cc8d1d14995b5dd643d7f0aabf082`; source fix `d2edb50411045b805e86828cd72025b79e7bdf31` maps to `c372c65f78b8e6a4cf91e5cf6eae2df5c0501418`; source validation documentation `504f802` maps to `a151357a68382f63a14da332a67b0d24daf3306b`.

At exactly c372c65, the integration owner reran `./scripts/check.sh` successfully, including fmt, strict all-target Clippy, tests, 3 doctests and strict rustdoc. `cargo test --workspace --all-targets --locked -- --list` reports 495 unique cases. Logs: `dev-c372c65-full.log` and `dev-c372c65-test-list.txt` in `/workspace/shared/minicore-host-validation`. `git diff --check main..HEAD` passed; all new dev subjects since f03aaac follow `type(scope): summary`, and SPEC SHA-256 remains unchanged.

Publication still waits for a newly reproduced, independent terminal-claim correction: failed Runtime report versus partial-save I/O, and prior cancellation versus later Runtime cleanup failure. These are real new probes, not failures of the hash fix. The separate ordinary-tool topic is also held for its own findings and reuse comparison and is not included in this dev checkpoint.
