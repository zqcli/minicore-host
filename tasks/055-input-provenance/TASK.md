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

Implementation and permanent regression tests are complete; final gate results are below. Required checks covered: formatting, clippy, real Host/Runtime/production HTTP/SSE source/reply/delegation/Loop/partial/restart/manual+automatic compaction regressions, unknown metadata and projection unit tests, workspace tests, documentation and unique test inventory. Scripted summary wording is not a live-model quality claim. No remote refs or publication will be changed.

### First implementation checkpoint

Coordinator-only shared hooks: `c42c1cedd8bef74cba4baedc0feb4f3508f2fe25` (4 files, 11 lines). Locked `cargo check -p minicore-host-core -p minicore-host-resources` passed. Five task055 pure core tests passed with `cargo test -p minicore-host-resources --test task055 -p minicore-host-core --lib --locked task055 -- --nocapture`; the explicit name filter ran only those five core tests, not the six differently named HTTP tests. HTTP tests have a separate unfiltered invocation and remain pending at this checkpoint. Logs are under `/workspace/shared/minicore-host-input-provenance-artifacts`; independent target is `/workspace/shared/minicore-host-input-provenance-target` (~614 MiB after this compile). Coordinator maintains the shared PROJECT.md.

### Permanent HTTP regression checkpoint

Production implementation and five pure tests: `53d84827716861b851cb041df55f9cccce28962b`.
`cargo test -p minicore-host-resources --test task055 --locked -- --nocapture` passed all **6** permanent HTTP cases (`http-initial.log`). These use the audit's request-gating mechanism and task042's shared model-response/configuration helpers, with production Host, Runtime, provider encoding, agent tool and persistent stores. The test wait helpers poll public retained facts without installing event/state subscriptions. Framework and Role occur once; source/reply text is compared with exact owner facts; manual utility's per-item input equals its original normal HTTP projection; raw native history and persisted prefixes/markers remain byte-exact through compaction and restart. Summary answers are explicit fixtures, not a live-model behavior claim.

The first all-target clippy invocation stopped on one test-only collapsible-if warning in the new compaction completion wait (`clippy-initial.log`). That wait is simplified without behavior changes; pure test one-item slices now borrow directly. `cargo clippy --workspace --all-targets --locked -- -D warnings` then passed (`clippy-final.log`). No production correction was needed.

### Aggregate assertion correction

The first `./scripts/check.sh` at `2debf2aa46d693fc4871067dfb0eced3f8fcb796` passed whitespace/format/clippy, then stopped in `crates/host-core/tests/http_host.rs`: two existing assertions expected HTTP user content to equal the unannotated raw text. The captured content instead had the intended metadata header followed by the complete original text. This was an expected test contract change, not native-history mutation or truncation. Original evidence is preserved in `full-gate.log` with status `101`; the later workspace/doc stages were not claimed passed.

The two existing tests now assert exact metadata against the actual accepted MessageId, user source and full original UTF-8 bytes, including the existing >262 KiB Unicode case. Existing native history, assistant response, Role and partial assertions remain. `cargo test -p minicore-host-core --test http_host --locked` passed all **3** cases (`http-host-adapted.log`). No production code changed after `53d8482`.

The second aggregate at `c6bed3e78b0668ba4e52955ffae7f434da0a553a` passed the adapted HTTP target, then stopped on four existing `runtime_owners` assertions whose shared `user_texts` helper still returned the full model presentation rather than the original input body (`full-gate-final.log`, status `101`). Later targets/docs were not executed by that stopped aggregate. Auditing every ModelMessage::User test consumer also found two scripted host-tools fixtures using plain task text or plain notification JSON. Their test-only parsing/classification is adapted; `scope.rs` and notification construction remain untouched.

All these test paths now share one small test-only decoder under `host-core/tests/support/input_context.rs`. It checks the exact header rendering, required valid MessageId/source, optional valid reply, and rejects extra/duplicate fields or null reply values before returning the complete untouched body. Tests assert actual user source, actual child notification source, and real root SessionRef/task MessageId for both agent.create and session.create delegation. The first consolidated helper compile used an unavailable direct serde import in the resources test crate; it was replaced with existing serde_json parsing and exact canonical comparison, with no manifest/dependency change (`strict-fixtures.log` preserves that compile-only failure).

`cargo test -p minicore-host-core --lib --test http_host --test runtime_owners -p minicore-host-tools --test host_integration --test delegation_acceptance_fences -p minicore-host-resources --test task055 --locked` passed **142** cases (`strict-fixtures-final.log`). This is targeted coverage, not yet a successful full-workspace gate. Production source remains unchanged from `53d8482`.

The third stopped aggregate at `60c2b8244447acf828cd7d4e0e14fa50d16015c5` reached resources task023: **11 passed, 5 failed** (`complete-gate.log`, status `101`). Inspection identified exactly three old whole-content equality/count assertions (task023.rs:551 and session_resource_resume.rs:83/144, expanded over both storage modes). Real requests and Turns had completed; the only mismatch was annotated model content versus bare original text. The repair keeps every resource/queue/configuration/binding assertion and now requires exactly one user message with the actual accepted MessageId, user source, no reply, and full original body via the same strict decoder. A repository-wide scan of ModelMessage::User and JSON content consumers found no additional plain-input presentation assumptions; remaining matches concern native input or framework/Role/assistant/tool content. The next workspace run uses `--no-fail-fast` to exercise every target in one pass rather than hiding later targets behind an early assertion stop.


## Final fixed-source verification

Code/test checkpoint: `8bd9bb7a6538e816ad663dbbbb4ef15d6bd608c9`. Subsequent task-record edits do not change the validated code. All commands use Rust 1.90.0 with `RUSTUP_HOME=/workspace/shared/rustup`, `CARGO_HOME=/workspace/shared/cargo`, Cargo bin first in PATH, `TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`, and dedicated `CARGO_TARGET_DIR=/workspace/shared/minicore-host-input-provenance-target`.

- `cargo test --workspace --all-targets --locked --no-fail-fast`: passed every target (`workspace-all-targets.log`, status `0`).
- `./scripts/check.sh` at the clean fixed checkpoint: **passed**, exit `0` (`final-check-8bd9bb7.log` and `.status`). This includes whitespace, workspace rustfmt, all-target clippy with `-D warnings`, full locked workspace/all-target tests, locked workspace doctests and workspace/no-deps rustdoc with `-D warnings`.
- `cargo test --workspace --all-targets --locked -- --list`: **790 unique test cases**, all **790** matched to successful execution in that final gate; **4 doctests** separately passed. No nested storage subprocess output is counted again. `test-inventory.log`, `verification-counts.json`.
- Unchanged-baseline red: exact `e3d634a2b0cd9e662d8efc26284c85af82edd8e5` in `/workspace/shared/minicore-host-input-provenance-red`, with only the final permanent HTTP test/helper files copied in. `cargo test -p minicore-host-resources --test task055 --locked same_text_user_and_peer_keep_actual_source_reply_and_bytes_across_restart -- --nocapture` fails at the absent input context, exit `101` (`red-final.log`, `.status`). Its independent target is `/workspace/shared/minicore-host-input-provenance-red-target`. The original audit and first red log are preserved.
- `cargo metadata --locked --offline --format-version 1` confirms the sole Runtime dependency remains `0.10.0` at `69faf9196991524b5549499d6208a1eaf567bf56`. `source-integrity-final.json` records a clean worktree and no SPEC/lock diff. SPEC SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`; Cargo.lock SHA-256 remains `151d9fb90f96072c9655f4f8a58ae4f4848f5caa2d1dc491fc2faa27ee2e79b8`.

The final gate includes the existing pending/failed-save, historical missing-tool-result, replay, ownership, queue, shutdown, cold-read and resource-rebinding suites as well as the 11 new provenance cases. The large HTTP test still checks complete >262 KiB Unicode input, Role and assistant output. No quotas, truncation, tool execution replay or observer-dependent source acquisition were introduced.

### Incremental commits

- `c42c1cedd8bef74cba4baedc0feb4f3508f2fe25`: coordinator's shared Session hooks.
- `53d84827716861b851cb041df55f9cccce28962b`: bounded model/utility provenance projection and five pure regressions.
- `9e32ea0a3441fe257539a93efdbc617753fb10ba`: six production HTTP regressions.
- `2debf2aa46d693fc4871067dfb0eced3f8fcb796`: test-only lint cleanup.
- `c6bed3e78b0668ba4e52955ffae7f434da0a553a`: existing large/ordinary HTTP expectations preserve exact original bytes and metadata.
- `60c2b8244447acf828cd7d4e0e14fa50d16015c5`: strict shared test decoder and legacy scheduler/delegation fixture adaptation.
- `8bd9bb7a6538e816ad663dbbbb4ef15d6bd608c9`: exact restored-input assertions in existing resource fixtures.

## Remaining boundary

Independent review and coordinator composition are owned by the parent task. All implementation/test changes are local; no push or remote ref change was made. SPEC, manifests, Cargo.lock, Runtime, TUI, production provider encoding, public DTOs, durable marker format and standard completion notification construction are unchanged. The latest main target is about 2.9 GiB and the independent red target about 463 MiB; shared disk had about 14 GiB free after verification. No other worker's cache was modified or removed. Requested Astra/max configuration and live model/TUI behavior remain explicitly unattested.
