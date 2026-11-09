# Implement model-backed compaction engine

Status: calculation module implemented; 21 focused tests and strict crate Clippy
passed. Full workspace gate and independent review are pending. Manual/automatic
Session compaction is not yet wired or claimed operational.

## Fixed inputs and ownership

- Host base: `04ba5bc07a7d9bb73eabb143839fbaeb264a0cea`.
- Topic: `feat/compaction-engine`; independent worktree and target directory.
- Parent bootstrap: `1e8caab63f126c9254d50467fe153fdd1bb46686`, cherry-picked here
  as `4911831`; workspace manifest, crate manifest and lock were parent-owned.
- Runtime: Git `69faf9196991524b5549499d6208a1eaf567bf56`, exact `0.10.0`, no patch.
- Reference Agent: `f36a87366d8e2fe0a1c8216463887902e471c739`; inspected
  compaction/projection/utility/threshold and Session compaction ownership.
  No recovery tickets, old 64 KiB summary, 512 KiB source, 64-group or 2000-char
  tool-result quota was copied. New prompts are original, not copied templates.
- SPEC v4.93 §§2/4/5/6/9/13 govern immutable binding, history/tool facts, ownership,
  cancellation and commit boundaries. SPEC and TUI are unchanged.
- Scope: new `host-compaction` sources/tests/README and this task only. No edits
  to Session, Host, production resource binding, existing prompt-history,
  transport, Runtime or storage schemas.

## Implemented boundary

`estimate_context` derives soft percentages from the real bound descriptor and
explicit Runtime context limits. `prepare` accepts immutable native history,
its revision, a protected suffix and the real one-message-per-item model request.
It selects an older prefix only at native assistant/tool-result boundaries.
`generate` invokes the real Runtime Model port with tools disabled and returns a
SummaryHistory candidate, coverage/source evidence, exact retained tail and
fixed request instructions, before/after estimates and per-call optional Usage.

Every source chunk contains complete exchange groups. Historical arguments and
outputs remain whole JSON data; missing results remain unknown. Opaque provider
metadata is not exposed as prose. Chunk targets use the current model context;
an oversized indivisible group is sent intact for actual provider adjudication.
Summary updates receive the previous summary. Stop + EOF, nonblank legal output
and a smaller estimated normal request are required. No utility retry is added.
The actual token/deadline reaches Model.start/stream, and cancellation/error/drop
cancels the utility child token. A response failure preserves observed usage.

The module does not mutate source history, own operation receipts/phases, save
anything or settle a commit/cancel race. The Session owner must still validate
original work/configuration, revision/content and cancellation at its write
boundary. Current `HistoryEntry::Summary` has no covered-prefix evidence and
cannot safely express durable replacement by simple append. That narrow storage
integration need was reported; this module does not unilaterally change it.

This first calculation layer only compacts a prefix. It does not fold disjoint
inside-Turn groups if all earlier input must remain protected, or force success
when source cannot be reduced. These limitations are explicit, not silent
history truncation or claimed manual/automatic acceptance.

## Verification so far

Rust 1.90.0, fixed Cargo.lock, two build jobs, independent target, debug info and
incremental compilation disabled. Commands:

```sh
cargo fmt --all
cargo check -p minicore-host-compaction --locked --offline
cargo test -p minicore-host-compaction --locked --offline
cargo clippy -p minicore-host-compaction --all-targets --locked --offline -- -D warnings
```

All focused checks pass: 19 explicitly named Scripted Model tests and 2 actual
Host provider TCP/HTTP/SSE tests (success/optional usage, cancellation/disconnect).
The Scripted Models exercise the real Runtime public types and are not represented
as live-provider or public Session execution. Coverage includes complete/missing/
mixed tools, native loop/request identity, preserved tails and source versions,
soft thresholds (not admission caps), oversized untruncated source, more than 64
source groups, summary text above 64 KiB, iterative summary updates, optional/
partial usage, malformed/empty/unfinished/tool responses, real cancellation and
deadlines, model change, model errors and no-progress failure.

Initial test compilation used outdated Runtime helper names and was corrected.
The first HTTP assertion incorrectly treated OpenAI prompt totals as Runtime's
uncached input field even when cached-token details were absent. The provider
correctly retained that category as unknown; the corrected test checks unknown
input and the actual provider total, with no production workaround.

Remaining: full workspace fmt/Clippy/tests/doctests/rustdoc and exact unique case
count, then independent review and Session/storage-owner integration. No push.
