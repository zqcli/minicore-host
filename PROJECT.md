# MiniCore Host

Implementation of the complete v4.93 design in `SPEC.md`. The first version as a whole is **not complete or release-ready**. The published foundation review checkpoint is [`82583b31ac9091164da5f6dcfee6bb23e05137e5`](https://github.com/zqcli/minicore-host/commit/82583b31ac9091164da5f6dcfee6bb23e05137e5) on `dev`, compared with remote `main` `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. It reproduces local source `797622fa9d663db01fad0d8b800a51a181ddc150`, whose code checkpoint was `275110695551c30582d1302e6b01ab23a3aad536`. [All source-to-published commit identities](docs/host-publication.json) are included for historical task records.

## Fixed baselines

| Component | Fixed source | Verification |
| --- | --- | --- |
| Design v4.93 | SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` | Full source unchanged; hash rechecked |
| Runtime upstream dev | `3fd5ecda1fb7c717ab0a39218d90268b4d97909c` | Reference baseline; original source lacks required Host handoff |
| Runtime published extension | `59ee8ce668f72a21d408e6247d3ce075b260db14` / `0.10.0` | `feat/host-history-handoff`; exact original trees; push CI 7/7 passed |
| Earlier Runtime handoff | `18a6ac9b82d003330cd870275efca793cdcc9a96` / `0.9.2` | Published ancestor, not the current Host pin |
| Agent reference dev | `f36a87366d8e2fe0a1c8216463887902e471c739` | Reference only, unchanged |
| TUI reference dev | `3beeb1d8a525e58ce6352e05061f242c5580e248` | Reference only, unchanged |
| Agent/TUI actual Runtime lock | `b276d9c0a1f3a3c06e32029f831ccd100c9699c7` | Not Runtime dev HEAD; these heads were not jointly tested |

## Implemented foundation

- `minicore-host-protocol`: strict IDs and direct-owner references, configuration/patch contracts, text-only requests, typed receipts/Faults, RPC mapping and the exact 52-method catalog. Unknown Fault leaves preserve their actual code, type, message and details; known mappings remain strict. A catalog entry is not an implemented handler.
- `minicore-host-storage`: Linux inode locks, owner-scoped object records, strict native Runtime JSONL, corruption rejection, exact accepted/persisted ranges and original failed-write reconciliation. Memory mode creates no files.
- `minicore-host-provider`: production OpenAI-compatible Chat Completions HTTP/SSE adapter, cancellation, conservative error/retry classification, authoritative partial metadata, original tool/history preservation and safe credential handling. Actual model constraints and parser safety remain; arbitrary application text, argument and error-body quotas do not.
- `minicore-host-core`: Host/Agent/Session owners, operation rights, independent queues, real Runtime execution, same-Turn continuation, frozen configuration bindings and canonical controls. Persistent startup preserves identity, changes incarnation, discovers direct owners and lazily loads history. Accepted creation and updates outlive callers. Necessary saves confirm before publication or execution continues. Graceful/force shutdown, Session-owned original-save continuation and ordinary memory reclamation are implemented.
- `minicore-host-transport`: 25 shared typed/stdio/WSS routes, concurrent framing, real TLS authentication, network-only key-change revocation and one client wait budget. The two asynchronous creation-validation callbacks do not block that budget. This is a library adapter, not a production Host binary, full configuration loader or completed event transport.
- `minicore-host-workspace`: real configured-root file browsing, UTF-8 reads/search, Git state/diffs and separately observed tool-edit snapshots. Canonical Session routing, actual write tools and persistent edit records remain integration work.

## Current verification

At fixed code checkpoint `2751106`, `./scripts/check.sh` passes whitespace, formatting, all-target workspace Clippy with warnings denied, **340 unique unit/integration tests**, **3 doctests** and rustdoc with warnings denied. `cargo test --workspace --all-targets --locked -- --list` confirms the unique count; nested storage subprocess output is not another test case. All four Runtime dependencies use the actual published Git revision, without a local patch, vendor copy or bootstrap source.

Tests use real owners and AgentLoop with explicitly scripted/gated model fixtures, real temporary files, real TCP/HTTP/SSE and real loopback TLS/WebSockets. Production-provider Host tests cover multiple Turns, payloads beyond removed quotas and authoritative truncated responses. These are genuine component/integration checks, **not live external model, production-process or TUI validation**. No user model account or live credentials were used.

A separate review of shared-client corrections at `5921c72` passed its full **331-test/3-doctest** gate, the original five probes, four new actual WSS probes and three repetitions of 29 focused cases, with no remaining scoped finding. It covers unknown Fault preservation, creation wait budgets, network revocation, TLS validation and large frames. Runtime 0.10 migration subsequently adds nine tests. Scoped review does not replace the requested external full-product review.

Exact commands, original topic-to-dev mappings, negative reproductions and historical checkpoints are retained in `tasks/*/TASK.md`. Earlier review found and fixed the HTTP-200 SSE Retry-After loss (`7b24ca2`), receipt lookup Fault conflation (`b961f9f`), Git query filter/classification defects (`775235a`), and the shared-client defects above. Earlier test passes are not claimed to have covered those missed cases.

## Reproducible Runtime dependency

All four Host manifests and `Cargo.lock` select [`59ee8ce668f72a21d408e6247d3ce075b260db14`](https://github.com/zqcli/minicore-runtime/commit/59ee8ce668f72a21d408e6247d3ce075b260db14), exactly version `0.10.0`. `cargo metadata --locked` confirms the Git source rather than a local path. The tree `a9affb5ec0a2c40dc68c77c1e0dec1a13bd2cc49` equals local source `cd59b6ecbad5ccfb67552660290992662253219b`.

The GitHub interface recreated seven quota-removal commits after the owner approved publication on the existing handoff branch. Original file trees, parent order and raw messages were verified. Published authors and committers are `zqcli`, with no co-author trailers. [Publication evidence and commit mapping](docs/runtime-publication.json) are included here. [Exact-head CI](https://github.com/zqcli/minicore-runtime/actions/runs/42619794415) passed all seven Linux, macOS, Windows and package checks. Runtime `dev` was unchanged; no Runtime `main` existed before or after publication.

This retains the earlier authoritative history handoff, partial reports and separate history failure. It removes arbitrary payload, tool-count/round and observation quotas while preserving protocol grammar, JSON parser stack safety, genuine model constraints, cancellation, deadlines and bounded retry. Authoritative retry-progress observation is a separate local Runtime extension under review/publication control and is **not** in this pin.

## First-version boundaries

- Complete the remaining canonical contracts and routes among the 52 methods, phased persistent deletion, production Host process/configuration/reload, full scoped events and output/history query synchronization.
- Integrate DSH delegation, built-in tools/workspace/write observations, standard interactions, compaction and legacy incomplete-tool-history projection. Independent topic work is not advertised as integrated until its owner tests, dependencies and aggregate gate pass.
- Perform actual TUI adaptation/terminal validation in its later stage, authorized live-provider validation, performance measurements and the requested external independent review.
- Necessary save failures cannot become successful `stopped`. Repeated shutdown returns facts, without implicit retry. Session resume confirms only its own retained originals. No new public recovery entry, witness framework or automatic repair is provided for an unpublished creation's failed save.
- Dropping the final root Host after an already-failed save without controlled shutdown is uncontrolled destruction, not successful retirement. No permanent self-cycle is added; ordinary accepted owners and explicit shutdown remain retained.
- Workspace Git preflight assumes stable local Git configuration and attributes. Uncoordinated external changes between preflight and Git execution are a known limitation; no atomic snapshot or cross-process lock is claimed.

## CI and GitHub handoff

The Ubuntu 24.04 workflow uses pinned Rust 1.90.0, an immutable official checkout action, full history, read-only contents access and no persisted checkout credentials. It calls the same gate. The published checkpoint passed the exact-head [Host CI run 42629644954](https://github.com/zqcli/minicore-host/actions/runs/42629644954), including the complete `./scripts/check.sh` gate. This is distinct from local test evidence. Actionlint/yamllint/shellcheck were unavailable and not run.

Local development uses topic worktrees and integrates onto `dev`. Local `main` remains `98621c9`. The verified remote `main` is the README-only `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. The authorized Host dev publication preserved that README as an ancestor and reconstructed all 101 local commits in order. Every raw commit message, tree and 511 referenced blob contents were verified; actual authors and committers are `zqcli <276883664+zqcli@users.noreply.github.com>`, without co-author trailers. GitHub API and independent `git ls-remote` agree on the published head. Remote `main` and other refs were unchanged; there was no force-push, tag or PR. [Task 028](tasks/028-host-publication/TASK.md) records the fixed handoff, mapping and hosted verification. The requested independent review and required tests still precede any main merge.

Workers were requested with `gpt-6-astra` and `max`. No independent runtime-model/effort attestation is exposed; the requested configuration is not claimed as proven actual identity.
## Scoped observation checkpoint

The source-owner observation stage adds typed scope/range DTOs, dynamic
connection-bound subscription dispatch, per-owner initial states and fixed
watermarks, UTF-8 catch-up reads, and Runtime event draining independent of
clients. Complete model/tool results come from the existing real history
handoff; process output remains separately owned by the original Session.
Optional tool output capabilities are bound at the actual invocation and expire
on return. A scripted gated model and a real temporary-file tool cover output,
scope, cancellation cleanup, internal Loop indexing and source reclamation.
This checkpoint does not claim finished interaction/retry projection, history
pagination, transport event routing or live-provider/TUI acceptance. See task 018.

A separate local observation checkpoint integrates authoritative Runtime retry
state, Host-format interaction identity mapping, real interaction settlement and
cold persistent history pagination. It is validated against the unpublished
Runtime retry-state candidate `3cf57e3e1947b0464bd8843ed0508bfddfb2639a`;
it is not part of the previously published Runtime pin or a claim that this
candidate is remotely reproducible. It also prevents unpublished Session
preparation from appearing as a registered child. Transport observation routing,
public interaction answers and all broader remaining product work remain open.

## Topic work: production resources and configuration (task 023)

The independent `feat/host-resource-configuration` topic adds real provider/tool/workspace composition, a single HostConfig persistence shape, immutable Turn bindings, credential-free resource queries and Host-owned accepted reload publication. It adds 12 focused tests and reaches 438 uniquely listed workspace tests plus 3 doctests on its fixed `57687b2` baseline. It does not yet complete transport reload/receipt routes, CLI startup, ordinary filesystem tools, compaction or the full first version. Enabled compaction is explicitly rejected rather than ignored. The known intermittent transport-close BrokenPipe remains reported even where a later complete gate passes. Exact scope, commands, failures and next-owner APIs are in `tasks/023-host-resources/TASK.md`; integration/review is separate from this topic checkpoint.
