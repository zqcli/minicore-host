# Task 077: Restore production tool approval

## Scope and fixed sources

- Start: `0072ca845780d25d889a490b48969e427f87fa52`, isolated `task/077-tool-approval` worktree.
- SPEC v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`; preserve unchanged. Section 13 retains existing approval in execution settings; section 15 decision 07 reuses existing Runtime interaction types without adding a platform.
- Runtime stays at `69faf9196991524b5549499d6208a1eaf567bf56`; no lockfile regeneration or TUI changes.
- minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739` is code reference, not a compatibility baseline. Reuse `profiles.rs` ApprovalMode/default/serde and `policy.rs` cancellation/deadline checks, three-mode decision table, Medium single-call approvals and safe name-mismatch denials. Host uses actual `AgentAction::parse` branches, excludes unimplemented apply_patch, and binds policy through its existing production Turn resolver rather than copying Agent Profiles/session setup.
- Worker requested as `gpt-6-astra` / `max`; this records the requested setup, not independent runtime attestation.

## Implementation in progress

Add `execution.approval` (`auto`, `ask`, `read_only`), default `ask` for omitted fields including old stored configurations. Execution replacement, persistence and next-Turn freezing remain with existing direct owners. Role carries no policy. Bind an immutable policy in `HostResourceBuilder`'s production resolver. Read/agent inspect/wait remain available; write/edit/bash and validated agent create/send/update/control require approval in ask and are denied in read_only. Registered external tools are conservatively treated as mutating: ask prompts, read_only denies and explicit auto allows. This deliberate departure from the reference name whitelist preserves Host static registry extensions without introducing a permission registry. Invalid agent actions and invocation/spec mismatches are denied in every mode. No new interaction API, approval cache, task table, limits, or Runtime change.

## Verification

Initial implementation and protocol/policy/schema checks pass: `cargo test -p minicore-host-protocol --test approval_config -p minicore-host-resources --lib -p minicore-host-tools --test contracts --locked --offline` passed 26 cases (including 9 new approval/config/schema cases) after `cargo fmt --all`. The function/config/schema wiring is one commit; no intermediate config-only behavior is published. Real binary and registered-external-tool acceptance remain in progress.

Existing non-approval ordinary/resource fixtures explicitly select Auto at their local constructors (`task042/support.rs`, inherited by task055 and task066; task023 probe setup; app tool_support). Their actual execution/counter/freeze assertions remain intact. The performance harness similarly explicitly selects Auto for its bash execution/cancel measurement, without changing measurement logic. Complete integrated stage gate and publication are owned by the parent; this topic does not push.
