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

### Production resource acceptance

- Function/config/schema source commit: `6feca08` (`feat(resources): bind per-turn tool approval policy`).
- `cargo test -p minicore-host-resources --locked --offline` passed **48 cases** and 0 doctests, including all existing ordinary-tool, input-provenance, credential and resource-freezing assertions plus four new registered-external cases.
- The external fixture is statically registered through `ToolRegistry` and counts real `Tool::execute` calls. Its real production resolver/Host/HTTP/SSE/Runtime chain proves Ask waits at zero calls, AllowOnce executes once, Deny and ReadOnly execute zero times, and Auto executes. Original Turn/Load/interaction IDs, read-only caller rejection and crossed/stale answer rejection are checked against actual owners.
- First integration compile used a nonexistent test-only Fault variant (`HostControlNotHeld`); corrected to the actual `HostAccessReadOnly` without production changes before the passing run.
- `python3 -m unittest discover -s scripts -p '*_test.py'` passed **20** pure checks. Adapted `scripts/task058_perf.py` SHA-256 is `f332e227d6c6094d5f2856c6ee75d3685e6937a782490f00ade844c089080373`; task074's older hashes remain historical evidence, not rewritten. No new performance result is claimed.

### Actual binary acceptance

- `cargo test -p minicore-host --test tool_approval --locked --offline` passed **6** compound process tests. Only loopback HTTP/SSE model replies are fixtures; the executable, production resolver, owners, Runtime, WSS/stdio, disk files and shell admission are real.
- Omitted approval is Ask through create/query/persistent reopen. Read executes without an interaction. Write has no file before approval; AllowOnce writes exactly once and a second call gets a different interaction. Deny leaves prior bytes unchanged. Original Host/incarnation/Agent/Session/Turn/Load/call/interaction identities and WSS observer rejection are asserted.
- ReadOnly blocks write/edit/bash and actual valid agent create/send/delegate/update/control invocations, including a self-policy escalation attempt. Files stay unchanged, no child objects or victim input/Turns appear, and victim lifecycle stays open. Read, inspect and wait execute successfully.
- Ask gates all four mutating agent action categories and Deny leaves actual child/object/message/title/lifecycle facts unchanged.
- Cancel and force shutdown during approval both exit without starting the real bash side effect; cancellation resolves the original interaction as cancelled.
- Changing Session policy from Ask to Auto does not replace the active Turn's policy, including a second internal Loop after a queued steer. Both old calls still ask, while the next public Turn executes automatically with the new config revision. Query projections verify the original/new bound policy values.
- Initial binary run passed 4/6; two new read fixtures omitted the existing required `limit` argument and received actual invalid-invocation output. Added `limit: 1` to those fixtures, without production changes; all 6 then passed.

### Initial scoped gate (before external-name provenance review)

With `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, `CARGO_TARGET_DIR=/workspace/shared/minicore-host-tool-approval-target`, `CARGO_BUILD_JOBS=2`, disabled dev/test debug and incremental compilation, and the shared temporary directory:

- `cargo test -p minicore-host -p minicore-host-protocol -p minicore-host-tools -p minicore-host-resources --all-targets --locked --offline`: passed **207** unique cases, confirmed by the same package/target selection with `-- --list` and unique target/test-name counting.
- `cargo clippy -p minicore-host -p minicore-host-protocol -p minicore-host-tools -p minicore-host-resources --all-targets --locked --offline -- -D warnings`: passed after replacing a new test fixture's default-plus-field reassignment with a struct initializer. The earlier Clippy failure is retained as a test-fixture lint issue, not a production failure.
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host -p minicore-host-protocol -p minicore-host-tools -p minicore-host-resources --no-deps --locked --offline`: passed.
- Formatting and whitespace checks pass. SPEC and Cargo.lock remain unchanged (lock SHA-256 `1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`).

Independent task078 review then reproduced an external factory named `read` bypassing Ask/ReadOnly via the name-only read exemption. This is a genuine conflict with conservative external-tool classification, not an OS sandbox claim. The initial passing scoped gate is retained; the issue remains open until the registry provenance correction and its probes pass. The analogous external `agent` name is included in that correction. No full workspace or release acceptance is claimed from this scoped gate.
