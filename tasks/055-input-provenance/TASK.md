# Preserve ordinary input provenance in model context

## Scope and baseline

- Exact base: `e3d634a2b0cd9e662d8efc26284c85af82edd8e5`. Isolated topic `task055-input-provenance`, worktree `/workspace/shared/minicore-host-input-provenance`.
- Fix the verified ordinary-input source/MessageId/reply omission in normal model requests and compaction utilities. The immutable audit at `/workspace/shared/input-provenance-audit-e30c29c/REPORT.md` and its five production HTTP characterization probes remain unchanged.
- Read AGENTS.md, the full relevant SPEC ownership, request assembly, persistence, input/Loop, compaction and delegation contracts, especially lines 144 and 2680. Frozen SPEC v4.93 and Cargo.lock stay unchanged. Runtime remains exactly `69faf9196991524b5549499d6208a1eaf567bf56`; TUI baseline `3beeb1d8a525e58ce6352e05061f242c5580e248` stays unchanged.
- Requested worker configuration: `gpt-6-astra`, `max`. Effective model/runtime settings are not independently attested.
- Agent reference `f36a87366d8e2fe0a1c8216463887902e471c739` is code reference only. Its request-local native projection and boundary identity approach were inspected, not its quotas or compatibility behavior imported.

## Bounded design

Session owns one sparse native-position index containing MessageId, MessageSource and in_reply_to only. The coordinator composes its small state/accept/load/reclaim hooks. Acceptance records from the exact Input entries in the existing atomic save boundary; validated replace_history rebuilds it locally before publication. Native-only/legacy history remains unknown. Partial and compaction-marker rows never consume native positions.

RequestHistory builds the metadata view in the same operations as optional summary, uncovered native suffix, base/appended split and actual unhanded appended inputs. Unhanded metadata uses existing bound_pending FIFO and verifies original Loop/input alignment; it never reads or consumes queues. Every native item remains one model message. Actual input text is retained verbatim after a compact data header in user role, including target IDs needed for reply references. Fixed framework/Role and native assistant/tool/replay projection remain unchanged.

The manual worker receives the same admission-time RequestHistory by value. Normal and manual preparation apply the same projection. CompactionPlan.original already owns the per-item prepared messages; utility serialization takes its exact user content at the same source ranges, preserving native kind/Loop facts without a second body or engine source cache. No new snapshot field, disk format, durable marker, public DTO, lifecycle, authority, observer dependency or Runtime contract is added. Existing source revision and atomic owner mutation cover the retained metadata. Standard completion notification construction is untouched.

## Verification

Implementation and permanent regression tests are in progress. Required checks: formatting, clippy, real Host/Runtime/production HTTP/SSE source/reply/delegation/Loop/partial/restart/manual+automatic compaction regressions, unknown metadata and projection unit tests, workspace tests, documentation and unique test inventory. Scripted summary wording is not a live-model quality claim. No remote refs or publication will be changed.

### First implementation checkpoint

Coordinator-only shared hooks: `c42c1cedd8bef74cba4baedc0feb4f3508f2fe25` (4 files, 11 lines). Locked `cargo check -p minicore-host-core -p minicore-host-resources` passed. Five task055 pure core tests passed with `cargo test -p minicore-host-resources --test task055 -p minicore-host-core --lib --locked task055 -- --nocapture`; the explicit name filter ran only those five core tests, not the six differently named HTTP tests. HTTP tests have a separate unfiltered invocation and remain pending at this checkpoint. Logs are under `/workspace/shared/minicore-host-input-provenance-artifacts`; independent target is `/workspace/shared/minicore-host-input-provenance-target` (~614 MiB after this compile). Coordinator maintains the shared PROJECT.md.
