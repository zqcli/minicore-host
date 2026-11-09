# Resume Sessions after resource repair

## Scope and contract

- Starting Host: `8b2b7449509521cb87f1025461c4d695c8f6adc7` on an isolated `fix/session-resource-resume` worktree.
- Test dependencies only: task023 `bfa802ef2489d6ac4926e3222eacb8f69feefaaf` and `a510d040f268d459ab2b1972088dfc1265577916`, locally cherry-picked as `d8623e8` and `9099f59`. The parent already owns their integration; cherry-pick only this task's later commits.
- Actual Runtime Git revision `69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0; no local override or dependency edit.
- SPEC v4.93 §§2, 4, 5, 13 and configuration acceptance matrix: pre-bind resource failure leaves accepted inputs queued, corrected configuration resumes that same queue, and explicit resume may recheck dependencies. History validation and original-save confirmation cannot be bypassed. The SPEC remains unchanged.
- Own only the Session Resume branch and successful configuration commit checks, plus focused tests and this task record. No Host reload, resource implementation, tool adaptation, current Turn binding, public recovery API, resend, automatic retry, or refresh loop changes. No push.

## Reproduction and implementation

The baseline rejects resume with any existing block, including a stale resource fault after Host repair. Successful configuration commits clear only missing resources, so preparation-time invalid configuration and Host-version conflicts permanently block the original input even after a valid update.

All four entry points use the same narrow set: `SessionResourceMissing`, `SessionConfigInvalid`, and `SessionConfigConflict`. Explicit resume lets the existing owner driver prepare the queued input once. Another preparation failure publishes its new real fault without consuming input. Successful configuration commits release only those startup blocks; explicit pause remains unchanged. Persistent resume still loads and validates history before any new binding, including when a failed-resource Session was unloaded.

## Test design and limits

- Core regressions use real SessionController ownership, the pinned Runtime and real journal/metadata files. A named resource-fault fixture counts preparations and injects only the three startup fault kinds; model output is scripted. Memory and persistent modes cover resume while still invalid, one explicit retry after repair, patch and candidate updates, rejected stale updates, explicit-pause preservation, and retained history read/write blocks.
- Production-resource regressions use HostResourceBuilder, actual Host configuration reload, real workspace binding and real HTTP/SSE over loopback. They cover restoring a removed model without modifying the Session, and correcting an invalid workspace through Session update in both modes. Persistent model repair also traverses unload/history-load/resume.
- No live external provider, TUI, or new compatibility/recovery platform is claimed.

## Commands and outcomes

Environment: source `/workspace/shared/minicore-host-resource-resume-evidence/env.sh`. It sets the shared pinned Rust/Cargo homes and temp root, jobs=2, debug info/incremental disabled, and an independent `CARGO_TARGET_DIR=/workspace/shared/minicore-host-resource-resume-target`.

- `cargo test -p minicore-host-core resource_resume --locked -- --nocapture`: baseline **6 failed, 2 passed**, with the six failures at stale resume/unchanged configuration blocks. Evidence: `red-core-confirmed.log`.
- Earlier fixture corrections are preserved in `red-core.log` (wrong MessageRecord field, compile failure) and `red-core-repro.log` (missing parent storage records, setup failures). They are not defect reproductions.
- `cargo test -p minicore-host-resources --test task023 session_resource_resume --locked -- --nocapture`: baseline **4 failed**, then **4 passed** after the fix. Evidence: `red-production-confirmed.log`, `green-production.log`. An earlier incorrect child-module path caused only a compile error (`red-production.log`), then was corrected before reproduction.
- `cargo test -p minicore-host-core --lib real_host_revision_conflict_clears_after_session_update --locked -- --nocapture`: **failed** with just the synchronous update fix reversed, then **passed** with that fix restored. The test gates actual Host resource preparation, publishes a newer Host revision, observes the real `SessionConfigConflict` and queued input, then commits a valid Session update. It does not inject that conflict. Evidence: `red-real-conflict.log` and the recorded patch.
- `cargo test -p minicore-host-core --lib resource_resume --locked -- --nocapture`: **9 passed**. All 13 new permanent tests now pass. Evidence: `green-core-final.log` and `green-production.log`.
- `./scripts/check.sh`: **passed** on production/test checkpoint `4f9739d01d30003faf978d53ff32ed16269251c4`, including whitespace, formatting, Clippy with `-D warnings`, all workspace targets, doctests and rustdoc with `-D warnings`. Evidence: `full-gate.log` (`FULL_GATE_EXIT=0`).
- `cargo test --workspace --all-targets --locked -- --list`: **464 unique target/test cases**, including this task's **13** new tests. Target/test pairs were deduplicated; nested storage subprocess output is not counted. Evidence: `test-list.log`, `test-count.txt`.
- **3 doctests passed**. No check was skipped or blocked. SPEC checksum remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`; manifests and lock are unchanged after the two declared task023 dependencies.

## Delivery (2026-11-09 UTC)

- Implementation and permanent tests: `4f9739d01d30003faf978d53ff32ed16269251c4` (`fix(core): revalidate session startup faults after repair`).
- A following documentation-only commit records the completed validation above.
- Only the implementation and documentation commits belong to this task; the parent should not replay the two dependency cherry-picks.
- Parent integration and independent review remain separate steps. No remote push was attempted.

Evidence directory: `/workspace/shared/minicore-host-resource-resume-evidence`.
