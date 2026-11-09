# MiniCore Host

Implementation of the complete v4.93 design in `SPEC.md`. The foundation is implemented and locally tested. The first version as a whole is **not complete or release-ready**.

## Fixed baselines

| Component | Fixed source | Verification |
| --- | --- | --- |
| Design v4.93 | SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` | Full source copied unchanged; hash rechecked |
| Runtime upstream dev | `3fd5ecda1fb7c717ab0a39218d90268b4d97909c` | Remote dev rechecked; original source lacks required Host handoff |
| Runtime published extension | `18a6ac9b82d003330cd870275efca793cdcc9a96` | `feat/host-history-handoff`; six verified commits, exact original source trees; push CI 7/7 passed |
| Agent reference dev | `f36a87366d8e2fe0a1c8216463887902e471c739` | Remote dev rechecked; reference only, unchanged |
| TUI reference dev | `3beeb1d8a525e58ce6352e05061f242c5580e248` | Remote dev/local checkout agree; clean and unchanged |
| Agent/TUI actual Runtime lock | `b276d9c0a1f3a3c06e32029f831ccd100c9699c7` | Not Runtime dev HEAD; no claim these heads were jointly tested |

## Implemented foundation

- `minicore-host-protocol`: strict IDs, direct-owner references, configuration/patch contracts, text-only requests, typed receipts/Faults, caller/RPC mapping and an exact 52-method catalog. Catalog availability is explicit and defaults to unimplemented.
- `minicore-host-storage`: Linux inode locks, owner-scoped object records, strict native Runtime JSONL, corruption rejection, exact accepted/persisted ranges and failed-write reconciliation. Memory mode creates no files.
- `minicore-host-provider`: real OpenAI-compatible Chat Completions HTTP/SSE adapter with cancellation, conservative error/retry classification, tool/history preservation and safe credential handling.
- `minicore-host-core`: Host/Agent/Session owners, connection operation rights, independent input queues, real Runtime execution, same-Turn continuation, fixed configuration bindings, controls and connection-scoped state observation. Persistent Host initialization preserves identity, creates a fresh incarnation, discovers direct owners and lazily loads JSONL. Accepted creation and metadata work outlive callers; confirmed configuration/default publication, durable close and explicit original-candidate reconciliation are implemented. Failed children and unrelated metadata candidates are isolated; all live descendants retain the directory lease. Full shutdown/deletion/reclamation and canonical service composition remain in progress.

## Verification boundaries

Host owner tests use a real AgentLoop with clearly named scripted model fixtures. Provider tests use real TCP/HTTP/SSE and the production transport against a loopback fixture. The full Host path has also completed two turns through that production adapter. These are genuine component/integration checks, **not live external model or TUI validation**.

No access key, live endpoint, payment, external message or user model account was used. TUI and Agent source were not changed. At Host checkpoint `d73b161`, the complete workspace gate passed 190 tests and 2 doctests, formatting, Clippy and rustdoc warnings-as-errors, using the published Runtime revision. The 16-case owner suite also passed 25 repeated runs, mostly using multi-thread Tokio scheduling; this is not a capacity measurement. Exact commands and outcomes are in `tasks/*/TASK.md`.

## Reproducible Runtime dependency

The approved Runtime extension is published at [`18a6ac9b82d003330cd870275efca793cdcc9a96`](https://github.com/zqcli/minicore-runtime/commit/18a6ac9b82d003330cd870275efca793cdcc9a96). The GitHub interface recreated the six original commits after the owner approved new commit identities and the interface's default authorship. Every blob, tree, parent order and raw commit message was verified against the original local history; all published authors and committers are `zqcli`, with no co-author trailers. The final tree is `46a3dc03a8ec0d89fc18ac39e9924eb1799ec45b`, identical to original local `4f976b41f2381e2b09f3383aa66318b7aee2a8b3`.

All three Runtime dependencies and `Cargo.lock` now select the published revision. The local Cargo patch and unused vendor symlink were removed; no bundle/bootstrap layer or copied Runtime source is used. `cargo metadata --locked` confirms the source is the GitHub revision rather than a workspace path. With this source, the full Host gate passed 178 unit/integration tests and 2 doctests before the last owner/storage corrections, then 190 tests and 2 doctests at `d73b161`. A fresh local Git clone of that fixed commit, with an empty build target and no ignored patch/vendor files, passed the same full gate. It shared the Cargo download cache, not source/build artifacts; this is not yet a remote Host CI result.

[Runtime push CI](https://github.com/zqcli/minicore-runtime/actions/runs/42598793185) completed successfully: Ubuntu quality gate, macOS and Windows tests, and all four package targets passed (7/7 checks). Runtime `dev` and `main` were not modified.

## Remaining first-version work

- Complete public Runtime history-save reconciliation, phased deletion, memory-object reclamation and Host-level lifecycle aggregation. Accepted creation, metadata/configuration publication, default/closed ordering and direct-owner discovery are implemented, not the whole lifecycle.
- Complete canonical service/receipt implementations for all 52 resource methods; the current core convenience controls are not full RPC implementations.
- Host shutdown/reload, network authentication/key publication, stdio and WSS, event scopes and output/read watermark synchronization.
- DSH delegation, complete built-in tools/workspace/diff support, standard interactions and compaction, and legacy incomplete-tool-history projection.
- Actual TUI protocol adaptation and end-to-end terminal validation, live provider validation when a suitable authorized setup is available, performance measurement and external independent review.
- Resolve the pinned Runtime's inherited structural/capacity limits against v4.93 Rule 12. Host adds no new application capacity quotas, but the upstream limits have not been removed.

## Branch and review policy

`main` remains the initial specification/documentation baseline (`98621c9`). Work is committed in reviewable modules on topic worktrees and integrated onto `dev`. The approved Runtime topic branch is published as recorded above. No Host push is authorized or attempted. Every push requires explicit approval; main merge additionally requires the requested independent review and required gates.

## Execution provenance

Workers were requested with `gpt-6-astra` and `max`. No independent runtime-model/effort attestation is exposed, so the requested configuration is not claimed as proven actual identity.
