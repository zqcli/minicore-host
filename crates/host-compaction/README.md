# Host compaction engine

The calculation layer for Host-owned compaction. Runtime is fixed at
`69faf9196991524b5549499d6208a1eaf567bf56` (`0.10.0`); it supplies the actual
Model port, request/response/history types and exchange validation, not a
compactor. This crate has no Session locks, filesystem access, public Turn,
control/input queue, retry service or durable operation state.

## Owner contract

1. Capture an immutable `HistorySnapshot` with the Session's history revision.
   Prepare the real model request with the already-bound model, reasoning,
   limits, tools and instructions. Its final messages must correspond one for
   one to the snapshot's history items. Use the existing Host prompt-history
   adapter for incomplete old tools; complete native calls/results stay native.
2. Use `estimate_context` to inspect estimated input and the configured soft
   trigger/target. `prepare` selects an older prefix, retaining at least the last
   whole exchange and the caller's protected suffix. An automatic boundary must
   protect newly appended inputs not yet handed to the Session owner; this
   is not a permanent fence at the public Turn's first input. Older accepted
   inputs and completed exchanges within a long Turn remain eligible. Manual
   callers can pass the history length.
   `None` means no eligible prefix. It neither changes history nor claims that
   context now fits the model.
3. Await `generate` outside owner locks with the bound `Model` and a real Runtime
   `ModelCallContext`. The supplied token and deadline reach each actual utility
   call. Each call's child token is cancelled at EOF/error/cancel/drop before
   another chunk starts; the caller's token is not cancelled. A
   successful model response must finish with Stop and EOF and contain valid,
   nonblank summary text after Runtime's standard raw-text normalization.
   Tool output, partial, length-limited, malformed and
   empty summaries never become candidates.
4. At the Session's ordinary write boundary, check the original work/configuration,
   cancellation and candidate source revision/content. `matches` checks only
   history evidence, not those other owner facts. Reconcile/commit/save there.
   The candidate's `history` and `request` are proposed projections, not a claim
   that anything was committed, saved or rolled back.

The utility sees full historical user text, tool arguments and outputs as JSON
user data, with tools disabled. Missing result records are explicitly unknown,
not invented failures or successes. Provider replay and opaque reasoning
artifacts are not exposed as prose or decoded; their presence is noted. The
immutable source retains all native records. Readable reasoning is included as
historical data; utility reasoning is not used as the final summary.

## Estimates, chunking and usage

Estimates count serialized Runtime request UTF-8 bytes / 4, including actual
fixed instructions/tools. They are not provider token counts or hard limits.
The context window is the smaller of the actual descriptor and an explicit
Runtime context limit; the existing `CompactionSettings` percentages derive the
trigger and target. A manual request need not exceed the automatic trigger.

The retained tail and summary share the soft reduced-context target. Complete
exchanges and protected inputs take precedence. Historical chunks use that
model's context window and contain whole exchanges, with each subsequent call
receiving the previous generated summary. A single oversized exchange is sent
intact, allowing the real provider to reject it. No text truncation, summary byte
cap, group-count quota, provider-overflow retry, recovery ticket or application
admission limit is added. Explicit Runtime output limits are forwarded unchanged;
soft targets never become injected output caps. A candidate must reduce the
estimated request, but need not meet the soft target or fit the context window.

`utility_usage` is a per-call list of actual optional Runtime `Usage` values.
Missing values and missing token categories remain unknown. Failed/cancelled
calls retain usage already reported before the failure; `completed` only means
that call ended with a validated Stop/EOF response. It does not mean complete
billing data. No byte estimate becomes usage and no retry is added around the
public Model port.

## Deliberate first-stage boundaries

This is a prefix compactor. It does not fold disjoint groups inside a caller's
protected suffix, so an all-protected history can have no eligible work. The
caller should protect unaccepted inputs, not every item of an active public Turn. Extremely
large indivisible input or a nonreducing summary yields no replacement, rather
than deleting source data to force success. Model errors retain their original
Runtime details. The Session owner decides how to report them.

This crate remains the pure candidate engine. Integrated Session ownership,
manual/automatic phases, operation receipts and cancellation live in
`host-core`; committed coverage lives in the storage `CompactionProjection`.
The native source history remains append-only, and only successfully saved
coverage changes the effective request history. Canonical typed/stdio/WSS
compaction controls are integrated; see tasks 036, 037 and 040 for their exact
checkpoints and reviewed save/cancel boundaries. This engine does not own
storage or change Runtime itself.

Tests use explicitly named Scripted Models and actual Host OpenAI HTTP/SSE
fixtures. Session and transport integration are tested separately. No live
provider credentials or TUI validation is claimed.
