# Retain partial history content

## Scope

- Starting Host: `d6485314680fee519574dfbb567b0906ad913c57`.
- Runtime: the actual Git dependency `69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0. No local Cargo patch, manifest, or lock changes.
- SPEC v4.93 §§4, 5, 9, 13 remains unchanged, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Fix the independently reproduced P1: saved Runtime partials were present in JSONL but absent from current and restarted public `session.history.read`.
- No TUI, transport route, save algorithm, original JSONL, execution recovery, queue, or delegation changes. No remote push.

## Implementation

- `HistoryItemData::Partial` explicitly carries unfinished content, incomplete tool names/argument text and usage. Like complete response reads, it omits internal Runtime request/replay metadata and does not fabricate current Turn/Load identities or a finish reason.
- `HistoryBackend::load` / `LoadFuture` now return original `Vec<HistoryEntry>` rather than the already filtered Runtime history. This is an intentional source-level trait change in the locally compiled Host. There is no compatibility fallback or synthetic record identity. The production Journal backend derives entries from the same validated `loaded_records` read; the empty Memory backend and all four test wrappers retain their actual behavior.
- The Session makes separate ordered projections: complete Runtime context and accepted public conversation content. Complete callbacks and formal partial report handoff extend readable content exactly once, before save confirmation. Saving and reconciliation never append the projection again. Partial content never enters model execution history.
- Unload preserves retained content; memory load preserves the actual in-memory projection. Validated persistent load replaces both projections together. A failed reload does not replace either with a prefix. Cold reads project actual validated records without creating a Runner, old Turn, or persistent cache. Session body reclamation releases the read projection too.
- Readability does not acknowledge persistence. Pending and failed saves remain pending/failed after reads, including a fully written but unconfirmed original range.

## Reproduction and focused checks

All checks use the actual pinned Runtime, real Host/Session owners and Runtime. Model event streams and write-failure gates are explicit test fixtures, not live-provider validation.

Environment:

```
RUSTUP_HOME=/workspace/shared/rustup
CARGO_HOME=/workspace/shared/cargo
PATH=/workspace/shared/cargo/bin:$PATH
TMPDIR=/workspace/shared/minicore-host-tmp
CARGO_TARGET_DIR=/workspace/shared/minicore-host-history-read-target
CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_TEST_DEBUG=0
CARGO_INCREMENTAL=0
CARGO_BUILD_JOBS=2
```

- Original review probe `saved_partial_remains_readable_in_public_cold_history`: independently red at the starting Host, then green after this fix. The temporary probe is archived outside the source tree and is not counted as a permanent test.
- `cargo test -p minicore-host-transport --test history_partials --locked`: passed. Real OS pipes cover cancelled UTF-8 output plus unfinished tool arguments with all subscriptions removed before generation; memory and persistent reads; page limits 1/2/3/100; partial followed by a complete Turn; unload/load; cold restart; no old Turn or Runner reconstruction; byte-identical JSONL across reads; and actual subsequent model requests excluding partials.
- `cargo test -p minicore-host-core session::tests::history_read --locked`: two tests passed. Actual Runtime reports and original Journal saves cover pending partial writes, zero/full-write failures, unchanged persistence on reads, and exact-once content/save after reconciliation.
- Development-only fixture correction: the first transport regression attempted an undeclared tool, and Runtime correctly rejected it before the intended partial. The fixture now registers that tool and panics if its unfinished arguments are ever executed. The corrected test passed.

Evidence directory: `/workspace/shared/minicore-host-history-read-evidence` contains the red/green original probe logs and focused-check output. Final workspace gate and exact unique test count are recorded after completion.

## Final validation (2026-11-09 UTC)

Production and permanent regression checkpoint:
`f67e9813e194eeec1b5c9c664d05fd9c3832c7c9`.

- `./scripts/check.sh`: passed, including whitespace, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --all-targets --locked`, `cargo test --workspace --doc --locked`, and `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`.
- `cargo test --workspace --all-targets --locked -- --list`: **389 unique target/test cases**. The three permanent partial-history tests are included; nested storage subprocess output and the archived temporary review probe are not additional cases.
- **3 doctests passed**. No test or documentation check was skipped or blocked.
- The exact Runtime Git source remains `69faf9196991524b5549499d6208a1eaf567bf56`; SPEC SHA-256 remains unchanged. `git diff --check` passed and no root manifest or lockfile changed.
- The independent reviewer received the frozen code SHA, red/green original probe, permanent tests, gate log, and test list for revalidation. Independent review and parent integration are separate completion steps; this task does not claim their result.

Final logs: `full-gate.log` and `test-list.log` in the evidence directory above.
No live provider, external network service, or TUI execution is claimed. The new
transport regression uses actual OS pipes; the existing workspace gate also
covers its existing in-process and loopback TLS routes.
