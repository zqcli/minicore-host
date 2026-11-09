# MiniCore Host

Implementation of the complete v4.93 design in `SPEC.md`. The foundation is implemented and locally tested. The first version as a whole is **not complete or release-ready**.

## Fixed baselines

| Component | Fixed source | Verification |
| --- | --- | --- |
| Design v4.93 | SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` | Full source copied unchanged; hash rechecked |
| Runtime upstream dev | `3fd5ecda1fb7c717ab0a39218d90268b4d97909c` | Remote dev rechecked; original source lacks required Host handoff |
| Runtime local extension | `49a6e8b06512616a2fb9c6104c07536b8396c4ca` | Four local commits; 307 tests, 2 doctests and full upstream gate passed; publication not authorized |
| Agent reference dev | `f36a87366d8e2fe0a1c8216463887902e471c739` | Remote dev rechecked; reference only, unchanged |
| TUI reference dev | `3beeb1d8a525e58ce6352e05061f242c5580e248` | Remote dev/local checkout agree; clean and unchanged |
| Agent/TUI actual Runtime lock | `b276d9c0a1f3a3c06e32029f831ccd100c9699c7` | Not Runtime dev HEAD; no claim these heads were jointly tested |

## Implemented foundation

- `minicore-host-protocol`: strict IDs, direct-owner references, configuration/patch contracts, text-only requests, typed receipts/Faults, caller/RPC mapping and an exact 52-method catalog. Catalog availability is explicit and defaults to unimplemented.
- `minicore-host-storage`: Linux inode locks, owner-scoped object records, strict native Runtime JSONL, corruption rejection, exact accepted/persisted ranges and failed-write reconciliation. Memory mode creates no files.
- `minicore-host-provider`: real OpenAI-compatible Chat Completions HTTP/SSE adapter with cancellation, conservative error/retry classification, tool/history preservation and safe credential handling.
- `minicore-host-core`: memory Host/Agent/Session owners, connection operation rights, independent input queues, real Runtime execution, same-Turn continuation, fixed configuration bindings, controls and connection-scoped state observation. The real-file journal adapter is exercised with real Session/Runtime saves.

## Verification boundaries

Host owner tests use a real AgentLoop with clearly named scripted model fixtures. Provider tests use real TCP/HTTP/SSE and the production transport against a loopback fixture. The full Host path has also completed two turns through that production adapter. These are genuine component/integration checks, **not live external model or TUI validation**.

No access key, live endpoint, payment, external message or user model account was used. TUI and Agent source were not changed. Exact commands and outcomes are in `tasks/*/TASK.md`.

## Local Runtime dependency blocker

The required Runtime extension is an independently committed, unpublished worktree. Local verification currently uses an ignored Cargo patch pointing to that worktree. The root lock reflects this local patch. A clean remote Host checkout is not yet advertised as independently buildable: the Runtime extension must be reviewed and published with explicit approval, then Host must pin that exact remote revision and regenerate/verify the lock without the local patch.

No bundle/bootstrap layer or copied Runtime source was added. The original Runtime git revision in manifests is provenance, **not the source used by the locally patched validation**.

## Remaining first-version work

- Persistent Host/Agent/Session startup and full lifecycle composition, save-reconciliation control, closed/default-pointer ordering and memory-object reclamation.
- Complete canonical service/receipt implementations for all 52 resource methods; the current core convenience controls are not full RPC implementations.
- Host shutdown/reload, network authentication/key publication, stdio and WSS, event scopes and output/read watermark synchronization.
- DSH delegation, complete built-in tools/workspace/diff support, standard interactions and compaction, and legacy incomplete-tool-history projection.
- Actual TUI protocol adaptation and end-to-end terminal validation, live provider validation when a suitable authorized setup is available, performance measurement and external independent review.
- Resolve the pinned Runtime's inherited structural/capacity limits against v4.93 Rule 12. Host adds no new application capacity quotas, but the upstream limits have not been removed.

## Branch and review policy

`main` remains the initial specification/documentation baseline (`98621c9`). Work is committed in reviewable modules on topic worktrees and integrated onto `dev`. No remote push has been performed. Every push requires explicit approval; main merge additionally requires the requested independent review and required gates.

## Execution provenance

Workers were requested with `gpt-6-astra` and `max`. No independent runtime-model/effort attestation is exposed, so the requested configuration is not claimed as proven actual identity.
