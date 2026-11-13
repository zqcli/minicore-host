# Task 109 — Responses provider work

## Checkpoints and ownership

- SPEC-first baseline: `32ea194ff2269aa8fb2bebff79355b600d404b45`, SPEC v4.10.2,
  SHA-256 `55476e6d2c6c9ec5b857f2b9235834a162b3ad85ce56a8e2a295135f7dc370c7`.
- Implementation: `64bcb814c571efa5e6dbd3c5c9f462210c25ed1f`
  (`feat(provider): add stateless OpenAI Responses transport`).
- Focused tests: `442e997356ae33b50cb43b5b3c5a20559c171e07`
  (`test(provider): cover Responses replay and failure boundaries`).
- Worker scope: `crates/host-provider` implementation, tests, README and provenance.
  Root owns HostConfig's required protocol selection, resource assembly, shared
  PROJECT/TASK state, final Cargo.lock/version update and publication.
- Requested worker configuration: `gpt-6-astra`, reasoning `max`. Effective
  runtime/model settings were not independently verified.
- Read-only Agent reference:
  `f36a87366d8e2fe0a1c8216463887902e471c739`; see provider PROVENANCE.md for the
  reused concepts, official wire references and intentional Host differences.

## Result

Added `OpenAiResponsesConfig` / `OpenAiResponsesModel`, retaining the existing
Chat Completions public API and safe shared HTTP send policy. The endpoint is
exact. Responses requests use complete explicit input, `stream=true`,
`store=false`, disabled provider truncation, native encrypted-reasoning inclusion
and only the output limit supplied by Runtime. Function definitions use the
Responses shape without modifying the Host's declared schema.

The typed stream decoder validates event order, output/content indices, item and
call identities, native completion snapshots, usage and terminal state. Completed
reasoning items preserve summary/content and encrypted data. Original message
IDs, phase, function item IDs, call IDs and item order survive history projection.
Canonical text ranges and streaming SHA-256/count evidence avoid a second
cumulative text or argument buffer. Ordinary UTF-8 runs are hashed in batches.
Runtime alone parses tool argument JSON and remains the only retry owner.

Only `response.completed` permits a complete response. Failed/incomplete/error/
EOF/cancelled attempts keep actual partial content and cannot execute tools.
Replay metadata precedes tool ends, so it also remains available if Runtime
rejects one argument after an earlier call validated. Native replay tags do not
establish success. Partial or foreign replay and inconsistent canonical content
fail locally instead of losing native history. No tool-argument observation API,
Runtime owner change or TUI change was introduced.

## Validation

Commands ran in the topic worktree with
`PATH=/home/agent/.cargo/bin:$PATH` and Rust/Cargo 1.90.0:

| Command | Outcome |
| --- | --- |
| `cargo check -p minicore-host-provider --offline -j 2` | Passed |
| `cargo test -p minicore-host-provider --all-targets --locked --offline -j 2` | 71 cases passed: 8 unit, 1 diagnostics, 42 Chat transport/Runtime, 20 Responses transport/Runtime |
| `cargo test -p minicore-host-provider --doc --locked --offline -j 2` | 2 compiling doctests passed |
| `cargo clippy -p minicore-host-provider --all-targets --offline -j 2 -- -D warnings` | Passed |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-provider --no-deps --offline -j 2` | Passed |
| `cargo fmt --all --check` | Passed |
| `git diff --check` | Passed |

The 20 new cases cover native encrypted reasoning with/without streamed summary,
phase replay after multibyte/control/CRLF normalization, original parallel call
IDs, canonical request projection, usage, Runtime-only malformed-argument
handling, terminal metadata ordering, repeated-snapshot disagreement, duplicate
recognized fields, event identity/order, retry classification and lower bounds,
pre-send/header-wait/stream cancellation, deadlines, failed/incomplete/EOF
partials, nonexecutable invalid headers, encrypted-only real output, empty/wrong
terminal rejection and incompatible replay rejection. Cancellation synchronizes
on actual Runtime text progress before cancelling.

The worktree lockfile has only the existing workspace-pinned `sha2 = 0.10.9`
added to the provider dependency list; root owns the final shared lock/version
commit. The focused tests use that resolved lockfile. Full workspace test counts,
integrated Host/compaction tests and independent review remain root acceptance
work and must be recorded in the shared task log before release.

## Limits

These are explicitly scripted loopback HTTP/SSE fixtures exercising reqwest,
the production adapter and the real Runtime ModelDriver. No live external model,
credentials, model quality/access or TUI integration was validated. The adapter
supports assistant text/refusal, reasoning and function calls; other native tool
or multimodal output and unsupported events fail explicitly. It requires the
documented item/content lifecycle and does not add permissive compatibility
fallbacks or server-side conversation state.
