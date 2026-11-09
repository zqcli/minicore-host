# Preserve delegation acceptance fences

## Scope

- Base: `713efb02319c1ff572cda0994cad2a106a8a1e71`.
- Actual Git Runtime `69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0, without a local patch. Root manifests and lockfile remain unchanged.
- SPEC v4.93 §§3, 5, 7, 13 is unchanged, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Fix two independently reproduced normal delegation failures: explicit-load completion losing discarded-input handoff, and appended steer consumed before the registered first follow-up losing its real shared Turn result.
- No TUI, transport, recovery platform, configuration, or remote publication changes.

## Explicit-load exit

- The original load owner now attempts necessary delegation handoff after releasing its state lock, on both success and failure, before its existing scheduler wake.
- Handoff retains the existing actual-execution/save/queue fences. Closing is not reported early, and load faults and partial-history projections remain unchanged.
- Original independent probes at the base reproduced both load failures and the steer failure (three red cases; the load-before-close control passed). After the load fix, the removal and load-before-close probes passed; the close probe's post-load `snapshot().unwrap()` instead observed correct memory reclamation. The archived green probe replaces only that post-close observation with actual handoff/join/reclamation checks. All four original probes then pass with the shared-range fix below.
- Four permanent current-thread Host/Runtime tests cover successful memory and failed persistent loads followed by close/removal before the accepted load runs. They assert real discarded inputs, no fabricated results, retained pause/fault and corrupt-file bytes, one notification, joined close, and actual memory Session/Agent reclamation.
- `cargo test -p minicore-host-core --test delegation_acceptance_fences --locked`: four passed. The model is a named gated fixture; owners, storage, and Runtime are production implementations.

## Verification

Evidence is held outside the tree at `/workspace/shared/minicore-host-delegation-evidence`.
Environment: `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, `PATH=/workspace/shared/cargo/bin:$PATH`, `TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_TARGET_DIR=/workspace/shared/minicore-host-delegation-target`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=2`.

## Shared activity qualification

- Commit `9e6f96ded7be397c5aa9019862e722acb903ff4c` contains the explicit-load exit and its four permanent gates.
- Real binding or consumption of any input already accepted by this delegation now starts its shared activity. A busy old Turn with no delegated input still cannot qualify. The unused initial-message consumption flag is removed.
- Each typed input range retains all real shared input IDs and additionally identifies the owner-confirmed delegated subset. The existing Session accepted set remains the only acceptance authority; the tool bridge only transfers this fact.
- Two additional core gates cover appended steer in T0 before the first follow-up in T1, and explicit removal of that first follow-up after the steer was consumed. Full shared outputs, input order, and a single notification remain intact.
- Six permanent core gates and all four archived independent probes pass. The close-probe observation adjustment above is recorded rather than misrepresented as an unchanged probe passing.

## Tool sink

- Commit `7c1facb0d7a37bdea336f3290f590b7d84a89b81` contains the shared-range qualification and typed bridge.
- A new test through actual Host, Runtime, `AgentToolFactory`, `CoreTurnPort`, ScopeManager and production Sink reproduced the first-follow-up-removal variant: M2 executes in T0 with a real `agent.inspect` ToolResult, M1 is explicitly removed, and the old sink rejects its real result. The other three tool variants passed before this adaptation.
- The sink now uses the owner-confirmed delegated input subset, validates matching child/Turn/result ranges and settled outcomes, and retains its one-handoff fence. It does not copy the accepted-message authority. Pure explicit removal still reports `not_executed` without fabricating a Turn.
- All four new real-tool cases pass after the sink fix: M2/T0 followed by M1/T1, first-input removal, failed T0 with retained inbox, and cancelled T0 with retained inbox. They assert real input order, ToolResults, original launch ToolResult unchanged, one notification, original parent, ordinary child retention, and explicit memory reclamation. Only the model is a gated test fixture.
- Existing no-delegated-input, cross-scope, discard-only and one-handoff tests remain in the scope suite; an additional explicit owner-port fixture checks mismatched child and input-range facts before a valid single handoff.

## Final local validation (2026-11-09 UTC)

Production and permanent regression checkpoint: `aa76ff9a9257d10abefc0fb8cc5dc3023f9b4e99`.

- `./scripts/check.sh`: passed, including whitespace, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --all-targets --locked`, `cargo test --workspace --doc --locked`, and `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`.
- `cargo test --workspace --all-targets --locked -- --list`: **450 unique target/test cases across 44 test targets**, including the eleven new permanent cases. The two identically named core/tools integration binaries are distinct targets. Nested storage subprocess probe output and archived temporary review probes are not extra cases.
- **3 doctests passed**. No check was skipped or blocked. No live provider or TUI behavior is claimed.
- `git diff --check` and SPEC checksum passed. Root manifests and Cargo.lock are unchanged; the actual pinned Runtime Git dependency remains version 0.10.0 at `69faf9196991524b5549499d6208a1eaf567bf56`.
- Final logs: `full-gate.log`, `test-list.log`, and `test-count.txt` in the evidence directory above. Original red/green probes, the genuinely red/green real-tool sink case, and focused core/scope runs are retained there.
- The independent reviewer received each exact code checkpoint and these evidence paths. Independent review and parent integration remain separate steps; this task does not claim their outcome. No commit was pushed.
