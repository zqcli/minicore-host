# Project saved incomplete tool history

## Scope and fixed sources

- SPEC v4.93 §4 and §15 pending 07. SPEC SHA-256 remains
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Host base: `b99afae2ff33222b9267c936ea73a8b25a8e0267`.
- Actual Cargo Git dependency: Runtime 0.10.0 at
  `69faf9196991524b5549499d6208a1eaf567bf56`. No local path override, manifest or
  lockfile change is used. `--offline` uses this already fetched remote revision.
- Agent reference inspected at `f36a87366d8e2fe0a1c8216463887902e471c739`;
  its old load-time newline/tail rewriting is deliberately not reused.
- TUI stays unchanged at `3beeb1d8a525e58ce6352e05061f242c5580e248`;
  its older locked Runtime is not the dependency tested here.
- Only the new core helper/export, task-specific tests and this task record are
  owned here. Session/controller persistence, config, transport and tools are
  not changed. No Git push is performed.

## Behavior

`prepare_prompt_history(PromptRequest) -> Result<PreparedPrompt, PromptError>`
first uses the real Runtime `DefaultPromptProvider`. Complete history keeps
exactly that output. Only unmatched calls in `HistoryView::base()` become
ordinary text at the same assistant-part positions. Each note retains the
original LoopId, request index and serialized ToolCall, including upstream ID,
name, arguments and original call index, and says that no corresponding result
record exists, not whether the tool ran or its outcome. Appended current-loop
calls are not treated as old history.

Matching is confined to the contiguous result exchange and requires the same
LoopId, request index, upstream ID and tool name. A later Turn's repeated ID
cannot complete an earlier call. Mixed records retain all other parts and real
results in their original order. Retained calls use dense request-local indices
because Runtime requires them; their upstream IDs/names/arguments are unchanged.
OpenAI Chat does not send this local index in its wire request.

Provider replay stays opaque and original. The actual production OpenAI Chat
encoder accepts its own finish/usage metadata through canonical parts and
explicitly rejects other replay formats or signed/encrypted reasoning before
HTTP. The helper does not display opaque metadata, invent a signature, or claim
compatibility with every provider. This task verifies both supported metadata
and explicit incompatible-data rejection against that actual adapter.

There are no invented ToolResults, old tool execution, recovery queues,
truncate/repair operations or history mutations. Native saved partials remain
partial records and never become complete model history. The framework, Role
and workspace context are still injected solely by the production resource
provider, not by this helper.

## Red-to-green evidence

Before implementing projection the helper was only
`DefaultPromptProvider::new(None).prepare(request).await`. The real persistent
Host successfully loaded the legal old JSONL with an unclosed call. Its native
history and exact file SHA-256 were unchanged and no tool or HTTP request ran.
Then the new input ended as `Failed` with `TurnInternalUnexpected` and message
`prompt preparation failed`, violating the same integration test's required
`Completed` assertion. The isolated real `ModelRequest::new` check reports
`IncompleteToolExchange`; that default rejection remains a passing diagnostic
regression, not a claimed successful continuation.

Red command, on the fixed Host/Runtime baseline:

```
cargo test -p minicore-host-core --test task024_prompt_history --locked --offline persistent_missing_result_runs_new_tool_once_over_real_http_without_observers -- --exact
```

After implementing the helper this same case and the complete/mixed cases pass
through actual Host -> JournalHistory -> Runtime -> OpenAiChatModel -> reqwest
-> TCP/SSE loopback. The endpoint is a scripted local fixture, not a live model;
the resource resolver and counting tool are explicitly test fixtures. No public
observation subscription is needed. Old native history and old JSONL bytes plus
SHA-256 remain identical, including a valid last record without final newline.
Only new work is appended; exactly the model's new call executes once.
Cancellation waits for a real Runtime output read, then saves the genuine partial
and cancellation facts without modifying the old prefix or running old tools.

The tests also cover full replay/content/result order, a large mixed assistant
with missing first/middle calls, reused upstream IDs in different loops,
current-loop strictness, cancelled/expired preparation, and unsupported opaque
replay and signed reasoning failing before network or tool execution.

## Validation environment and status

```
RUSTUP_HOME=/workspace/shared/rustup
CARGO_HOME=/workspace/shared/cargo
PATH=/workspace/shared/cargo/bin:$PATH
CARGO_TARGET_DIR=/workspace/shared/minicore-host-prompt-history-target
TMPDIR=/workspace/shared/minicore-host-tmp
CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_TEST_DEBUG=0
CARGO_INCREMENTAL=0
CARGO_BUILD_JOBS=2
```

- Targeted task024 suite: 11 tests passed.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- Full workspace/all-targets, unique `--list` count, doctests and strict rustdoc:
  running at the source checkpoint; final outcomes follow in the validation
  record commit.

Development-only failures were fixed before the meaningful red run: initial
fixture code used the wrong MessageSource variant and an ambiguous Vec `as_ref`;
the saved Agent initially referenced its Session before that Session existed.
Clippy required a lexical scope rather than explicit `drop` of a test MutexGuard
before an await. None of these failures was reported as passing verification.

## Production integration boundary

The resource worker must replace its existing default-history call with
`minicore_host_core::prepare_prompt_history(request).await?` in its production
BoundPrompt. Its framework/Role/workspace injection remains the sole owner.
Until that change and integration validation are complete, this task only
claims the tested helper and real-component acceptance fixture, not completed
Host process/configuration integration.

The existing `HistoryEntry::Input::runtime_item()` retains only Runtime's
UserHistory (LoopId, kind, text), not MessageId/source/in_reply_to. This existing
input-provenance boundary was reported to the parent and resource worker; this
narrow task does not add a parallel context bus or change Session data types.
The parent owns the corresponding PROJECT.md and aggregate integration update.
