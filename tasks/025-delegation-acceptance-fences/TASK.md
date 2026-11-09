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

The real agent-tool sink adaptation/regression, complete workspace gates and unique test count are pending. Independent review is a separate step and is not claimed here.
