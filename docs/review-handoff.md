# Foundation review handoff

## Review target and authority

The target is the `dev` foundation, with provider-correction checkpoint `38991d37bc4980fc31a3312e943a62a060ce635a`, not a complete v4.93 release. The full specification remains the implementation target; unimplemented surfaces are not waived by this review. `main` remains the initial specification/documentation commit `98621c9` and must not receive development changes before the requested external independent review and required checks pass.

Read `AGENTS.md`, the complete `SPEC.md`, `PROJECT.md`, and `docs/method-coverage.md` first. The unchanged specification SHA-256 is `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`. The 52-method catalog describes names/contracts, not 52 finished handlers. No remote Host publication or main merge has been performed. Remote pushes require the owner's explicit approval.

This handoff is self-contained in the repository. The review must not depend on another conversation, an unpublished Runtime worktree, hidden test data, or a prior review's conclusions.

## Dependency and integration identity

- Runtime: published `zqcli/minicore-runtime:feat/host-history-handoff`, exact revision [`18a6ac9b82d003330cd870275efca793cdcc9a96`](https://github.com/zqcli/minicore-runtime/commit/18a6ac9b82d003330cd870275efca793cdcc9a96). It adds authoritative incremental history handoff, partial reports and separate history failure without replacing a primary execution failure.
- Runtime's [push CI run](https://github.com/zqcli/minicore-runtime/actions/runs/42598793185) passed all seven checks. Its source tree is `46a3dc03a8ec0d89fc18ac39e9924eb1799ec45b`, equal to the original local handoff tree. The six-commit identity mapping is in task 001.
- Every Host Runtime dependency and the sole root `Cargo.lock` use that remote revision. No active local Cargo patch, copied Runtime source or bootstrap bundle is required.
- Agent reference `f36a87366d8e2fe0a1c8216463887902e471c739` and TUI reference `3beeb1d8a525e58ce6352e05061f242c5580e248` are unchanged. Both actually lock older Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`; their heads were not jointly tested with this Host.

## Implemented paths worth tracing

1. Protocol: strict generated IDs and direct-owner references, text-only inputs, configuration/patch validation, Fault registry, typed receipts and JSON-RPC contracts. Look for invalid combinations accepted by deserialization or misleading method availability.
2. Storage: Linux directory-inode ownership, fd-relative no-follow operations, independent child discovery, exact object identities, strict native-history JSONL and durability/reconciliation ranges. Check that neither a readable file nor accepted bytes are mislabeled as confirmed persistence.
3. Core: Host -> Agent -> SessionController -> real Runtime. Follow both input queues, Turn binding, same-Turn continuation, cancellation claim ownership, finalization and separate execution/save failures. No public event stream is used to reconstruct persistent model content.
4. Persistent owners: initialization, owned asynchronous creation, partial receipts, configuration/default publication after confirmed metadata saves, explicit original-candidate reconciliation, lazy history loads, durable close ordering and stop aggregation. Child notifications carry no parent mutation authority; live memory descendants still retain the root lease.
5. Provider: real OpenAI-compatible Chat Completions HTTP/SSE, conservative delivery/retry classification, original tool IDs, authoritative partial content, and non-executable incomplete tool-name/ID fragments retained as provider partial metadata. It is not the old Agent's Responses adapter.

Pay particular attention to acceptance versus publication, owner retirement versus a caller's completed wait, configuration revision versus other state revisions, actual current/pending default references, and the distinction between a completed Runtime execution and a durably finalized public Turn.

## Reproducing checks

Use Linux with the pinned Rust 1.90.0 toolchain, rustfmt and Clippy. From the repository root:

```sh
cargo metadata --locked --format-version 1
./scripts/check.sh
sha256sum SPEC.md
```

The script requires full Git history and checks committed, staged and unstaged whitespace before formatting, all-target workspace Clippy with warnings denied, unit/integration tests, doctests, rustdoc with warnings denied and diff whitespace. The Linux CI workflow calls this same script with read-only repository permissions and a pinned official checkout action. The first hosted Host run is still pending publication; local success is not a hosted-runner result.

Test evidence is actual code execution, with its boundaries kept explicit:

- `host-core/tests/runtime_owners.rs`: real owners and real AgentLoop with scripted/gated model fixtures; most tests use multiple Tokio worker threads.
- `host-core/src/session/tests.rs`: real temporary JSONL and save barriers, cancellation during a save, authoritative partial and simultaneous execution/save failure.
- `host-core/tests/persistent_owners.rs`: actual object records, restart, corruption isolation, owned creation/configuration, default/close dependencies, root lease and metadata concurrency.
- `host-core/tests/http_host.rs`: two turns through Host, Runtime and the production provider over actual loopback HTTP/SSE.
- Storage tests: real filesystem/process locking, abrupt process exit, unsafe paths, corruption, partial writes and sync/reconciliation failures. Metadata gates coordinate only the same file's raw snapshot/publication, not owner state or unrelated files.
- Provider tests: production HTTP/SSE transport plus real Runtime. No live external model credentials or endpoint were used.

The final clean-source checkpoint at `0b9b390` passed the entire 198-test/2-doctest gate in a fresh local Git clone with its own empty target directory, using the published Runtime Git source. An earlier clean clone at `d73b161` passed its then-current 190-test/2-doctest gate. Cargo downloads were cached; this was not a remote Host clone or hosted workflow. Those clean-source counts are core 67, protocol 47, provider 27 and storage 57. A later independent review found the HTTP 200 SSE Retry-After defect; fix `7b24ca2` and nine new real HTTP regressions are integrated at `38991d3`, whose complete gate passes 207 tests and 2 doctests (provider 36). The original defect was reproduced before fixing it; a separate provider-scoped review passed the fixed regression and additional probes. The earlier 198-test result is not evidence that this missed case was covered. Seven additional default-reference, revocation and gate-reacquisition regressions from targeted review are part of those counts. A separate fresh-target review passed core/storage tests and lint/doc checks; it is not the requested external review. Exact commands and source checkpoints are recorded in task 001 and task 006.

## Required scope still open

- Complete canonical service implementations and receipts for all 52 methods, typed/stdio/WSS equivalence, shared client timeout ownership and actual Host process integration.
- Host shutdown/reload and execution-resource configuration; HostSettings currently contains only name metadata. Network authentication, key changes and full scoped/filterable events are not implemented.
- Owner-level continuation of failed Runtime history saves, phased object deletion and automatic memory-entity reclamation. The lower-level storage reconciliation API alone does not complete these owner controls.
- Output byte ranges/watermarks, DSH delegation, complete built-in tools/workspace/diff support, standard interactions, compaction and legacy incomplete-tool-history projection.
- Other provider-specific adapters/compatibility modes, live authorized model validation, actual TUI integration and performance measurements.
- Reconcile inherited Runtime structural/capacity ceilings with v4.93 Rule 12; the Host has not silently removed or validated those inherited limits.

Do not convert these gaps into fake-success handlers or treat DTOs, fixture responses, repeated test runs, local targeted review or Runtime CI as evidence of full Host release acceptance. The requested external independent review is still required. Findings should name the exact commit and source/contract, give a reproducer when possible, and distinguish an implemented-path defect from a declared unfinished surface without excusing either for the final release.
