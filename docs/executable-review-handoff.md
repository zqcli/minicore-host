# Executable checkpoint review handoff

## Latest deletion composition

Fixed local code `4187899ef9b7301ed5af4cf36b84b5825fb83bed` adds the eleven reviewed task051 commits without source conflicts. The complete `./scripts/check.sh` passes **811 unique tests and 4 doctests**; locked `--list` confirms 88 nonempty targets. Formatting, all-target Clippy with warnings denied, strict rustdoc and committed whitespace pass. [The new manifest](executable-deletion-verification.json) records exact log hashes and dependency/SPEC verification. Prior manifests remain unchanged.

Both deletion methods now use the actual owners through typed/stdio/WSS. Explicit deletion closes only when requested, retains immutable admission receipts, and waits for real cleanup/save/result handoff before removing owned data. Independent review reproduced and then cleared two P1 defects: capability/scope fences happened after receipts, and an unbound memory input could survive Closing and strand deletion. The unchanged ten independent probes and six permanent fence regressions passed at source `4b379623faa2398a00efe8a456d7c6085c65ffc3`. Four-package review ran 488 passing cases with only two known old adapter expectations failing; those expectations were already corrected in this integration, whose normal aggregate is fully green. Review did not claim a whole-workspace run. The [source map](executable-source-map.json) now preserves 63 topic commits.

All **52 catalog methods are routed**, but this is not full contract completion: the separately tracked query envelopes, bound configuration and original terminal revision corrections are still under implementation/review. Input provenance projection, complete business diagnostics, measured performance and external review also remain open. This fixed local checkpoint is **not published**. Last verified remote `dev` is still `a84e36f32c9ab7340ac06f5fb3a2cc47d3ab0f0f`; main and Runtime are unchanged.

## Earlier cold-history composition

Local code `e3d634a2b0cd9e662d8efc26284c85af82edd8e5` adds the four reviewed task052 commits to the earlier fixed checkpoint below, without conflicts. `./scripts/check.sh` passes **779 unique cases and 4 doctests** on the first combined run. The new [verification manifest](executable-cold-history-verification.json) records the independent inventory, sole Runtime pin and exact log hashes. The source map now contains 52 topic commits. The deterministic original active-writer failure and independent 276-case/8-probe review are preserved; the old 761-case evidence below remains valid for its named source. No remote publication is claimed. Other listed findings remain open; cold-read coordination alone is now resolved in this local composition.

## Fixed checkpoint and publication

- Local code: `70c17e7226bd90c7bb1e146ed141081f567e9d23`, integration topic `stage/executable-next`; compare local published-source baseline `810e3817a78855871d13f8127a3f0e87db821d66`.
- Last verified published Host dev: `a84e36f32c9ab7340ac06f5fb3a2cc47d3ab0f0f`, mapping to that baseline. Its exact-head CI 42653342131 succeeded. **The new code is not yet published**, so these local hashes are not presented as GitHub handoff links.
- Remote main remains the README-only `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. No main merge or forced update.
- The earlier execution integration/source maps remain unchanged. [Executable source mapping](executable-source-map.json) adds 48 granular topic commits; original raw messages/order are verified. Two local fixture-integration corrections are separately recorded in task 048.

## Scope and engineering choices

Actual Host process/configuration/TLS/stdio startup and shutdown are assembled. Production read/write/edit/bash share the frozen workspace, actual per-call worker cleanup, Session output/edit ownership and strict UTF-8/binary-prefix observations. Six workspace queries, six remaining canonical controls and context use the same typed/stdio/WSS owners, bringing actual routes to 50. Context remains a pure owner query: unknown cold history is absent, never fabricated empty or loaded as a query side effect. It reports actual committed/bound resources and compaction/save facts.

The independent diagnostic and storage lease findings were caused by open file descriptions inherited through ordinary concurrent fork. Their fixed original owners explicitly release the final OS lease only after required work, retaining delayed-save/cleanup behavior. Context review also corrected public internal-Loop identity leakage and made received history/body/cursor/original save registration one owner boundary. No queue, alternate history database, persistent rescue API or compatibility platform was added.

Tasks 042–050 record relevant topic scope; tasks 047/049 retain the original lease red tests and fixed review evidence. Independent scoped reviews passed for production tools, process startup, workspace source validation, controls, context, both lease fixes and actual binary tool acceptance. The four permanent binary tool cases passed 40 repeated executions; three extra independent real-binary probes passed 30. Those probes are concrete supplementary evidence, not additions to the 761-case committed inventory.

## Actual fixed-source validation

Environment: Linux, Rust 1.90.0, pinned Cargo.lock; jobs=2, debug information/incremental compilation disabled. Normal parallel test behavior is retained.

- `./scripts/check.sh`: pass at exact `70c17e7`. Includes committed/worktree whitespace, rustfmt, workspace all-target Clippy with warnings denied, all tests, four doctests and strict rustdoc.
- `cargo test --workspace --all-targets --locked -- --list`: **761 unique target/test-name pairs**, 84 nonempty targets. Nested storage subprocess output is not counted again.
- `cargo metadata --format-version 1 --locked`: sole Runtime exactly 0.10.0, published Git `69faf9196991524b5549499d6208a1eaf567bf56`; no local patch/vendor/bootstrap.
- `git diff --check main..HEAD`: pass at fixed code; SPEC SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Logs: `/workspace/shared/minicore-host-validation/executable-context-process-70c17e7-full.log`, `executable-70c17e7-test-list.txt`, `executable-70c17e7-metadata.json`. [Verification manifest](executable-verification.json) preserves hashes and counts.

The first previous combined run failed two stale unsupported-agent.start fixture expectations, corrected in `1bf0969`. The next composition failed the context fixture's older constructor arity; `70c17e7` identifies its newly created record with reopened=false. Neither correction changes production semantics or removes business assertions. Original failed logs remain recorded in task 048; no failed aggregate is relabeled as passing.

## Open findings and remaining work

1. Independent public-contract probes after this green gate found missing full bound configuration in turn.get, including finished Turns after Session config changes. Actual execution uses frozen resources correctly; queries still need the original bound value.
2. message.get, turn.list, turn.result and turn.wait need the required Host/incarnation/target query envelopes; result/wait must use public content projections instead of exposing internal Runtime history fields. These findings are not declared resolved by the gate.
3. Cold session.history.read/load coordination has a separate reviewed-candidate fix, not included in this hash. Deletion's two owner-backed routes are implemented in a separate topic awaiting independent review/integration.
4. Ordinary peer/delegated input source and reply context is persisted but missing in normal model and compaction requests. Standard completion notifications already contain their source/task/original-call/results and should remain intact. A narrow Session-owned projection is planned.
5. Actual business diagnostics and host.info health remain under implementation. The process pipeline itself is installed and tested.
6. Complete current 52-method contract audit, measured performance/resource checks and requested external review remain. Loopback HTTP/SSE fixtures are real adapter/Runtime integration, not live remote-model validation. TUI is unchanged and untested against the new Host.

## Companion repositories

Runtime `feat/host-history-handoff` at published `69faf9196991524b5549499d6208a1eaf567bf56` passed CI 42624211012 (all seven jobs). Upstream Runtime dev remains reference `3fd5ecda1fb7c717ab0a39218d90268b4d97909c`. Agent `f36a87366d8e2fe0a1c8216463887902e471c739` is code reference only; TUI `3beeb1d8a525e58ce6352e05061f242c5580e248` is unchanged. Both actually lock old Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`; no joint Agent/TUI integration is claimed.

Next: fix the confirmed query projections/revisions, finish provenance/diagnostics work, rerun the complete combined fixed-source gate, then publish only the authorized Host dev checkpoint with per-tree mapping and exact remote CI. A published stage is still not approval to merge main.
