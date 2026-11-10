# Historical first-version contract audit

This report applies to the fixed source listed below. Its open findings describe
that checkpoint; see [current project status](../PROJECT.md) for subsequent fixes,
validation and remaining work. Source-file links use published checkpoint
`2615fe00`, whose tree matches the audited tree recorded below; later file moves
do not redirect this historical evidence to current code.

## Decision and fixed source

**Not full first-version acceptance.** All 52 canonical methods have actual
shared routes at the source below. Their implemented contracts and focused
regressions are mapped here, not converted into a completion percentage.
One newly reproduced **P1** remains: `turn.result` cannot read a valid saved
Turn result after Host restart. The known Runtime retry-wait elapsed field,
complete business diagnostics, fresh performance acceptance and external final
review have separate owners. TUI and live-provider acceptance are not claimed.

| Item | Audited value |
| --- | --- |
| Host source | `587318b69a2e1f72d0f82999d3f6f6badacfe0e4` |
| Source tree | `03aad0d4af62a0b89fadcb3d3dc78d63d12425b9` |
| Coordinator-reported equivalent remote `dev` | `2615fe00ff385a74c6ef6a302c169ccc2b6b4410`; this audit did not fetch, push or reverify remote/CI state |
| Specification | Immutable `SPEC.md` v4.93, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` |
| Runtime | Git `69faf9196991524b5549499d6208a1eaf567bf56`, version `0.10.0`; not later Runtime work |
| Requested reviewer configuration | `gpt-6-astra`, `max`; effective runtime settings not independently attested |

This is a new audit, not recovery of the lost task075 artifacts. It preserves
[method-coverage.md](method-coverage.md), whose older `76a7bfb` table is historical.
Later diagnostics, Runtime changes or fixes are outside this fixed-source verdict.

### Evidence rules

- **I/F** means the implementation was traced to its real owner and the named
  committed test's relevant assertions were inspected. It does **not** mean this
  audit reran that test, proved every race or approved the entire method forever.
  The final column identifies the actual boundary or missing branch.
- **G1** is the fresh failing real-owner restart probe below. A route and passing
  same-run tests do not discharge this saved-result requirement.
- The coordinator reports a fresh fixed-baseline gate of **908 unique Rust
  cases, 4 doctests and 20 Python cases**, plus strict lint/docs/fmt. This audit
  did not rerun that full gate or obtain a new CI result. CI `42741046081` was
  last reported in progress, not passed. Counts are evidence inventory, not
  contract percentages, and apply only to the named source.
- This audit independently read the complete SPEC, production paths and focused
  test content, and ran only the new restart probe twice. Exact commands,
  failure evidence and document checks are in [task086](../tasks/086-final-contract-audit/TASK.md).
  Scripted/gated models in committed tests are explicit fixtures over real
  Runtime; local HTTP/SSE tests exercise the production provider but are not
  live external-model runs.

## Route and source key

[The catalog](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-protocol/src/catalog.rs) defines 52 names.
[operations.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-transport/src/operations.rs) supplies 51 ordinary
`Operation::execute` routes; `events.subscribe` is the explicit stream route in
[client.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-transport/src/client.rs) and
[rpc.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-transport/src/rpc.rs). Local calls invoke the same typed
operation directly; stdio/WSS decode, execute and encode that operation. A
catalog entry or `Implemented` capability is route availability, not acceptance.

In the matrix, `H` = Host, `A` = its direct Agent, `S` = that Agent's direct
SessionController. Every `H -> S` abbreviation includes `H.agent -> A.session`;
there is no global Session lookup or selected-page fallback. Owner source keys:

| Key | Actual production source |
| --- | --- |
| H | [host.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host.rs) |
| HC / HO | [host/controls.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/controls.rs) / [host/observation.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/observation.rs) |
| HD / HM | [host/discovery.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/discovery.rs) / [host/metadata.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/metadata.rs) |
| HR / HW | [host/configuration.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/configuration.rs), [configuration.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/configuration.rs) / [host/workspace.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/workspace.rs) |
| HCreate / HStop / HDelete | [host/create.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/create.rs) / [host/shutdown.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/shutdown.rs) / [host/deletion.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/deletion.rs) |
| A / AD / AM | [agent.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/agent.rs) / [agent/discovery.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/agent/discovery.rs) / [agent/metadata.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/agent/metadata.rs) |
| ACreate / ADelete | [agent/create.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/agent/create.rs) / [agent/deletion.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/agent/deletion.rs) |
| S / SC / SM | [session.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session.rs) / [session/controls.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/controls.rs) / [session/metadata.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/metadata.rs) |
| SQ / SO | [session/observation/query.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/observation/query.rs) / [session/observation.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/observation.rs) |
| SH / SX / SP | [session/history_read.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/history_read.rs) / [session/context.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/context.rs) / [session/compaction.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/compaction.rs) |
| SI / SE / SD | [session/interaction.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/interaction.rs) / [session/tool_edits.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/tool_edits.rs) / [session/deletion.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/deletion.rs) |

Test references below are exact `file::test_function` names. `CT:` expands to
`crates/host-core/tests/`; `TT:` expands to `crates/host-transport/tests/`.
They identify test content, not a fresh execution claim.

## Complete 52-method acceptance matrix

### Host (5)

| Canonical method | Actual route / owner | Key contract and exact focused test | Status / boundary |
| --- | --- | --- | --- |
| `host.info` | `H.info` -> HD direct-owner facilities/capabilities | Actual identity, operator state, owner summaries: `TT:task071_discovery.rs::task071_discovery_pages_and_details_match_direct_and_stdio` | I/F. Summary is current owner facts, not full config/body or product acceptance. |
| `host.config.get` | `H.config_get` -> HR configuration snapshot | Current committed config/revision, no credential values: `TT:resource_queries.rs::readonly_queries_share_real_config_models_and_explicit_tool_selections` | I/F. Configured credential-source references are not secret values. |
| `host.reload` | `H.reload` -> HR owned prepare/publication | Whole candidate, preparation faults and restart-only fields: `TT:reload.rs::all_transports_retain_preparation_faults_and_whole_candidate_restart_reasons` | I/F. Accepted receipt is not publication; key-rotation boundary is checked separately below. No plugin hot-update promise. |
| `host.receipt.get` | `H.receipt_get` -> HC direct-owner receipt collections | Missing versus duplicate-ID ambiguity: `TT:adapters.rs::receipt_lookup_faults_are_not_method_unavailable` | I/F. Operation IDs correlate independent invocations, not idempotency/replay; receipts do not retain released source owners. |
| `host.shutdown` | `H.shutdown` -> HStop `run_shutdown`, A/S cleanup/save owners | Accepted force retains save/root ownership after caller and Host handle drop: `CT:host_shutdown.rs::force_keeps_owned_save_and_directory_after_callers_and_host_handle_drop` | I/F. Controlled shutdown is owned until actual cleanup/save; receipt alone is not stopped. Crash durability or a last-Arc self-cycle is not claimed. |

### Agent (8)

| Canonical method | Actual route / owner | Key contract and exact focused test | Status / boundary |
| --- | --- | --- | --- |
| `agent.create` | `H.create_agent_async` -> HCreate, then ACreate | Preserve published Agent when default Session fails; no false message attempt: `CT:persistent_owners.rs::default_session_real_file_failure_keeps_published_agent_receipt`; `TT:adapters.rs::partial_create_and_accepted_start_error_remain_full_rpc_results` | I/F. Partial success is returned, not rolled back or hidden by RPC error. No new unpublished-creation recovery entry. |
| `agent.get` | `H.agent_details` -> HD -> `A.info`, AD `session_page` | Actual details plus lightweight directly owned Session summaries: `TT:task071_discovery.rs::task071_discovery_pages_and_details_match_direct_and_stdio` | I/F. Details may contain config; summaries intentionally do not. No global-child snapshot promise. |
| `agent.list` | `H.list_agents` -> HD -> AD `summary` | Exclusive lexical cursor, absent limit means all: `CT:task071_discovery.rs::agent_pages_are_exclusive_lexical_and_none_returns_all` | I/F. Concrete `{items,next_cursor}` is correct; no universal identity/data wrapper requirement. |
| `agent.update` | `H.agent_update` -> HM -> AM `update_control` | Committed config survives failed metadata write, immutable acceptance receipt: `TT:public_controls_storage.rs::failed_metadata_saves_keep_committed_config_and_report_fault_without_rewriting_receipt` | I/F. Defaults affect later Sessions, not existing bound Turns; accepted save fault remains explicit. |
| `agent.start` | `H.agent_start` -> HC -> `A.set_enabled` | Current operation rights; start does not clear explicit Session pause: `TT:public_controls.rs::typed_all_six_controls_use_real_owners` | I/F. Resumes permissible scheduling only, not closed/deleting objects or hard blockers. Same shared operation backs pipe/TLS tests. |
| `agent.stop` | `H.agent_stop` -> HC -> `A.set_enabled`, child cancel with inbox retained | Preserve queues and report stopping/stopped facts: `CT:runtime_owners.rs::canonical_agent_stop_preserves_queues_and_formed_lifecycle` | I/F. Stop is not close/delete, and repeated receipt is not a new cleanup guarantee. |
| `agent.set_default_session` | `H.agent_set_default_session` -> HM -> AM `set_default_control` | Exact directly owned pointer; null means none, no fallback: `TT:public_controls.rs::typed_all_six_controls_use_real_owners` | I/F. Persistent Agent records only reloadable default; memory default is runtime-only. Close/default-save ordering is separate save evidence below. |
| `agent.delete` | `H.agent_delete` -> HDelete -> ADelete explicit Session cascade -> SD | Unknown artifact blocks removal and preserves true partial progress: `CT:task051_deletion.rs::unknown_nested_artifact_blocks_removal_and_reports_real_partial_cascade` | I/F. Only explicit directly owned data; no recursive workspace/artifact deletion or global rollback. |

### Session (12)

| Canonical method | Actual route / owner | Key contract and exact focused test | Status / boundary |
| --- | --- | --- | --- |
| `session.create` | `H.create_session_async` -> HCreate -> ACreate `create_session_async` | Real P/P, P/M, M/M creation; M/P rejected; partial initial-input failure: `CT:persistent_owners.rs::persistent_creation_modes_reopen_and_real_partial_input_failure` | I/F. Parent/Session mode and creation steps remain independent; no memory reconstruction on restart. |
| `session.get` | `H.session_info` -> `S.info` | Explicit target metadata/effective config: `TT:adapters.rs::remaining_query_and_canonical_control_routes_return_real_owner_facts` | I/F. Not lightweight execution state, not an implicit active Session selector. |
| `session.list` | `H.list_sessions` -> HD -> AD `session_page` | Filter before pagination; durable explicit close reason survives reopen: `CT:task071_discovery.rs::session_filters_precede_pagination_and_close_reason_survives_reopen` | I/F. `{items,next_cursor}`; reclaimed memory entries are absent, not empty revived objects. |
| `session.update` | `H.session_update` -> HM -> SM `update_control` | Failed save leaves committed title/config/revision, fault and original receipt: `TT:public_controls_storage.rs::failed_metadata_saves_keep_committed_config_and_report_fault_without_rewriting_receipt` | I/F. New config binds next Turn; current Turn/continuation keeps original resources and approval. |
| `session.state` | `H.session_state` -> `S.snapshot` | Lightweight current identity/state separate from original terminal Turn revision: `TT:task070_query_recovery.rs::finished_transition_owns_a_revision_and_all_queries_keep_it_during_later_turn` | I/F. No full Role/body; ordinary source missing/reclaimed Fault, not fake empty state. |
| `session.load` | `H.session_load` -> HC -> SC `control` / SM owned load | Loading acceptance is immutable; load does not resume paused input: `CT:runtime_owners.rs::canonical_session_controls_freeze_actual_acceptance_facts` | I/F. Full valid history precedes executable context; no old execution/queue restoration. G1 is not fixed by loading. |
| `session.unload` | `H.session_unload` -> HC -> SC `control` | Residency drops while queue and pause facts remain: `CT:runtime_owners.rs::canonical_session_controls_freeze_actual_acceptance_facts` | I/F. Active/preparing/compacting/saving work blocks unload; memory history is not discarded merely by unloading. |
| `session.pause` | `H.session_pause` -> HC -> SC `control` | Explicit pause is independent of loaded residency and receipts: `CT:runtime_owners.rs::canonical_session_controls_freeze_actual_acceptance_facts` | I/F. Stops later consumption, not cancellation of current Runtime or already accepted saves. |
| `session.resume` | `H.session_resume` -> HC -> SC `control`, SM history load/reconcile | History validation cannot be skipped with empty queues: `CT:persistent_owners.rs::review_resume_validates_history_without_new_input` | I/F. Original failed save/load is reconciled; cannot reopen closed Session or invent retry of unrelated work. |
| `session.cancel` | `H.session_cancel` -> HC -> SC `cancel` without explicit Turn selector | Idle queue clear versus exact active cancellation: `CT:runtime_owners.rs::canonical_session_cancel_reports_idle_and_exact_current_turn` | I/F. `keep_inbox` is explicit; cancellation is not pause or proof that cleanup/save completed. |
| `session.close` | `H.session_close` -> HC -> AM `close_session` -> SM close/save -> retirement | Returned accepted close facts remain true after memory reclamation: `CT:runtime_owners.rs::canonical_close_returns_accepted_facts_even_when_memory_is_reclaimed` | I/F. Persistent explicit close is durable; no reopen. Clear exact default before close commit; no data deletion implied. |
| `session.delete` | `H.session_delete` -> HDelete -> ADelete `delete_session` -> SD | Real owner deletion on typed/pipe/TLS routes: `TT:deletion.rs::typed_delete_uses_real_owners`, `TT:deletion.rs::os_pipe_stdio_delete_uses_real_owners`, `TT:deletion.rs::tls_wss_delete_uses_real_owners` | I/F. Admission fences new work; explicit close option; required saves/cleanup precede owned-record removal. Independent artifacts survive. |

### Messages and input (4)

| Canonical method | Actual route / owner | Key contract and exact focused test | Status / boundary |
| --- | --- | --- | --- |
| `message.send` | `H.send` -> exact A default or explicit `A.session` -> `S.send` | Two queues consumed once in their actual order: `CT:runtime_owners.rs::initial_batch_uses_two_independent_queues_once` | I/F. Acceptance is not Turn start/completion. No implicit resend on wait timeout; configured default absence is a Fault. |
| `message.get` | `H.message_snapshot` -> HO -> SQ -> `S.message` | Current Host/receiver identity, real provenance/queue/binding: `TT:task054_query_contracts.rs::message_get_must_carry_current_host_and_receiver_identity` | I/F. Only retained message facts; not a cross-restart message index or evidence that missing input never executed. |
| `input.list` | `H.input_list` -> HC -> SC `input_list` | Separate next-step/next-turn order and user provenance: `TT:public_controls.rs::typed_all_six_controls_use_real_owners` | I/F. No global FIFO and no active bound input reclassified as queued. |
| `input.remove` | `H.input_remove` -> HC -> SC `remove_control` | Exact queued MessageId and optional Controller revision; independent duplicate operations: `TT:public_controls.rs::typed_all_six_controls_use_real_owners` | I/F. Bound or already removed input cannot be recalled; no later input clear or automatic redelivery. |

### Turn (6)

| Canonical method | Actual route / owner | Key contract and exact focused test | Status / boundary |
| --- | --- | --- | --- |
| `turn.get` | `H.turn_snapshot` -> HO -> SQ `turn_details` | Original bound config despite Session update: `TT:task054_query_contracts.rs::turn_get_must_return_original_bound_config_after_session_update` | I/F for retained execution. Restart has no old TurnInfo; no separate complete historical execution reconstruction is required by this audit. |
| `turn.list` | `H.turn_list` -> HO -> SQ completion order plus active | Original finished revision/Load across later Turn and same-run unload/reload: `TT:task070_query_recovery.rs::persistent_unload_reload_preserves_original_terminal_revision_and_load` | I/F for retained list. Fresh restart list is empty even with saved history; not evidence that no Turn previously ran. This is not a saved-result acceptance test. |
| `turn.wait` | `H.wait_turn_snapshot` -> HO weak state wait -> SQ result | Target identity and original result: `TT:task054_query_contracts.rs::turn_result_and_wait_must_carry_current_host_and_original_target`; timeout isolation: `TT:adapters.rs::timeout_and_dropped_wait_do_not_cancel_or_resend_accepted_work` | I/F for retained Turn. Wait timeout is not cancellation or DSH completion; restart target currently missing, not silently resumed. |
| `turn.cancel` | `H.turn_cancel` -> HC -> SC exact `cancel` | Stale/ending target never clears later input or retargets next Turn: `CT:runtime_owners.rs::canonical_turn_cancellation_never_retargets_or_clears_later_input` | I/F. Inbox clear occurs only at the successful claim boundary; cancellation outcome remains distinct from save/cleanup diagnostics. |
| `turn.result` | `H.turn_result_snapshot` -> HO -> SQ `state.completed` | Same-run identity/config/complete and partial records: `TT:task054_query_contracts.rs::turn_result_and_wait_must_carry_current_host_and_original_target` | **I/F, missing saved branch: G1/P1.** Private `persisted_turn_result_remains_queryable_without_restoring_execution` fails after true restart and after explicit load. |
| `turn.output.read` | `H.turn_output_read` -> HO original Turn source -> SO `output_read` | UTF-8 fixed ranges and completion before tail: `CT:scoped_observation.rs::completion_can_precede_tail_events_without_losing_original_source_bytes` | I/F for actually retained source. Not a permanent output store; cold history remains readable without rebuilding old output buffers. G1 does not require that rebuild. |

### Data, workspace and interactions (15)

| Canonical method | Actual route / owner | Key contract and exact focused test | Status / boundary |
| --- | --- | --- | --- |
| `model.list` | `H.model_list` -> HR current configuration resources | Complete configured references/capabilities, no provider call: `TT:resource_queries.rs::readonly_queries_share_real_config_models_and_explicit_tool_selections` | I/F. Query is not a model-health/live-provider guarantee; known references can execute without listing first. |
| `tool.list` | `H.tool_list[_for_agent/_for_session]` -> HR registry/committed selection | Actual registered versus selected declarations: `TT:resource_queries.rs::readonly_queries_share_real_config_models_and_explicit_tool_selections` | I/F. Production builtins are agent/read/write/edit/bash; no required apply_patch or old Agent compatibility. Unknown removed selection is a Fault, not silent empty support. |
| `session.history.read` | `H.history_read` -> HO -> SH / persistent `HistoryBackend` | Saved cold content identical with/without observation; read creates no Runtime: `TT:observation_tools.rs::observed_and_unobserved_runs_save_identical_history_and_cold_rpc_reads_do_not_create_runtime` | I/F. Native complete/partial/summary projection and real cursor; terminal marker is not a public history item. That omission does not satisfy `turn.result` (G1). |
| `session.context` | `H.session_context` -> SX locked read projection | Cold history is unknown, not empty, and load/unload preserve actual known values: `CT:session_context.rs::cold_history_is_unknown_until_explicit_load_and_retained_across_unload` | I/F. Read-only, no model/resource preparation or implicit load. Usage is actual complete-response usage, not fabricated estimates. |
| `session.compact` | `H.session_compact` -> HC -> SP `compact`, owned provider/save | Real history compression without Turn creation and preserved paused queues: `TT:compaction.rs::all_transports_compact_real_history_without_turns_and_preserve_paused_queue_order` | I/F. Busy rejects rather than enqueues; acceptance is not finished/save-confirmed; actual summary replacement coverage only. |
| `session.compact.cancel` | `H.session_compact_cancel` -> HC -> SP `compact_cancel` | Exact original operation, later work and both queues untouched: `TT:compaction.rs::all_transports_cancel_exact_original_without_clearing_queues_or_later_work` | I/F. Cancel request has its own operation ID; no retargeting or business outcome inferred from waiter cancellation. |
| `workspace.files` | `H.workspace_files` -> HW captured Session workspace -> `host-workspace` | Explicit target, actual paginated directories across all transports: `TT:workspace_queries.rs::readonly_clients_read_the_explicit_session_and_utf8_pages_on_all_transports` | I/F. No client-page fallback, message auto-copy or filesystem isolation claim. |
| `workspace.read` | `H.workspace_read` -> HW captured workspace -> `host-workspace` | Actual UTF-8 file bytes/ranges: `TT:workspace_queries.rs::readonly_clients_read_the_explicit_session_and_utf8_pages_on_all_transports` | I/F. Current requested file, not an immutable artifact archive; missing/unsafe path Fault remains distinct from empty data. |
| `workspace.search` | `H.workspace_search` -> HW captured workspace -> `host-workspace` | Actual matching paths and continuation: `TT:workspace_queries.rs::readonly_clients_read_the_explicit_session_and_utf8_pages_on_all_transports` | I/F. Explicit query scope and real filesystem failures; no speculative search service or global quota. |
| `workspace.status` | `H.workspace_status` -> HW -> actual Git/filesystem state | Real repo changes versus nonrepo: `TT:workspace_queries.rs::git_status_and_diffs_are_actual_workspace_facts_and_keep_live_cursor_semantics` | I/F. Corrupt repo/unsupported filter remain faults, not a successful empty repo. |
| `changes.list` | `H.changes_list` -> HW workspace Git or SE retained edit journal | Workspace-unknown origin separate from Session edits; lexical continuation: `TT:workspace_queries.rs::git_status_and_diffs_are_actual_workspace_facts_and_keep_live_cursor_semantics` | I/F. Reopened Session edit coverage is explicitly missing, not rebuilt from filesystem guesses. |
| `changes.diff` | `H.changes_diff` -> HW live workspace or SE original edit record | Real UTF-8 diff pages; changed live source invalidates old cursor: `TT:workspace_queries.rs::git_status_and_diffs_are_actual_workspace_facts_and_keep_live_cursor_semantics` | I/F. Original tool edits remain source-bound; no arbitrary reconstructed historical diff. |
| `toolcall.get` | `H.toolcall_get` -> HO -> SO `tool_get` | Exact arguments, progress/state, final native result and source: `TT:observation_tools.rs::real_file_tool_arguments_progress_output_and_result_cross_os_pipes_and_tls` | I/F. Retained original ToolCall only; later delegation association does not reopen/rewrite its launch result. |
| `toolcall.output.read` | `H.toolcall_output_read` -> HO -> SO `output_read` | Original tool channel UTF-8 bytes separate from complete result: `TT:observation_tools.rs::real_file_tool_arguments_progress_output_and_result_cross_os_pipes_and_tls` | I/F. No invented progress/transcript from final result; released source Fault, not permanent output retention. |
| `interaction.answer` | `H.answer_interaction` -> HO -> SI original pending interaction | Exact Turn/Load/interaction, replacement operator, one settlement and preserved pause: `TT:interaction_answers.rs::real_answer_survives_operator_replacement_preserves_pause_and_retains_runtime_result` | I/F. Only current operator can answer; deadline/cancel and original Runtime answer types remain authoritative, not a new Role permission system. |

### Observation (2)

| Canonical method | Actual route / owner | Key contract and exact focused test | Status / boundary |
| --- | --- | --- | --- |
| `events.subscribe` | Special `HostClient.subscribe` / RPC `serve_frames` -> HO `events_subscribe` -> connection observation registry plus S baselines | Real Host/Agent/Session scope, later descendants, identity, watermark/history: `TT:observations.rs::typed_pipes_and_tls_share_real_scoped_push_snapshots_and_history` | I/F. Setup has one call deadline; established stream does not. Source data belongs to owners, not subscription; no observer required for save or delegation handoff. |
| `events.unsubscribe` | ordinary `Unsubscribe` -> `H.events_unsubscribe` -> connection observation registry | Read-only observer ends only its own stream; source reclamation remains possible: `TT:observations.rs::readonly_observers_only_cancel_their_own_stream_and_source_can_be_reclaimed` | I/F. No business cancellation or closure of another subscription's shared connection. |

## G1 / P1: saved Turn result disappears at Host restart

### Requirement and concrete failure

SPEC §8, line 1415, defines `turn.result` as:

> 读取该 Session 实际持有或已保存的 Turn 结果

SPEC §4, line 364, separately says:

> 已保存历史、结果和明确关闭事实继续有效

The prohibition on restoring old execution/queues/tools (§4 line 360 and §9
line 1738) does not erase this read-only saved-result branch. The failing probe
uses real owners, pinned Runtime, the production OpenAI-compatible provider over
one actual loopback HTTP/SSE fixture, and real persistent files:

1. Complete one persistent Turn; require actual `Completed` and `Persisted`.
2. Gracefully stop; verify JSONL has the original TurnId's `TurnFinished` with
   `Completed`, alongside actual input and complete response (three records).
3. Reopen the same root: same HostId, new incarnation, no active execution.
   `session.history.read` returns the saved answer without changing residency.
4. `turn.result(original_id)` returns `SESSION_TURN_NOT_FOUND` / **3201**;
   `turn.list` is empty. Explicit `session.load` finishes, but both observations
   remain unchanged. JSONL bytes remain identical and no old work runs.
5. The final assertion requiring that real saved result fails. Both runs
   fail the same assertion; this is not a hypothetical storage race.

Exact fresh test: `persisted_turn_result_remains_queryable_without_restoring_execution`.
The temporary source and logs are described in task086; the failing probe is not
added to production or the committed suite by this documentation-only audit.

### Source path that loses the queryable result

- [host-storage records.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-storage/src/records.rs), lines 136-159:
  `Input`, `Runtime` and `Partial` retain TurnId; runtime/partial records retain
  response index. `TurnFinished` retains TurnId, actual outcome and reason.
- [session/history.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/history.rs), around lines
  460 and 531-550, persists the terminal fact and places the live result in
  `state.completed` during actual finalization.
- [SQ](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/observation/query.rs), lines 80-105,
  `turn_result_snapshot` reads only `state.completed` (line 88). There is no
  validated saved-record read path. `retained_turn` likewise checks only active
  and completed maps; `turn_list` uses the retained completion sequence.
- [SH](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/history_read.rs), lines 5-20,
  `HistoryItemData::from_entry` does not expose `TurnFinished`. Its `replace_history`
  (lines 24-65) loads model context/provenance/readable history, not a result
  projection. [persistent.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/session/persistent.rs)
  constructs a new Session without the old execution maps. That absence of old
  execution is correct; the absent saved-result reader is not.

### Minimal repair constraints, not a production change

Prefer a **read-only saved-result projection** from the Session's validated
existing records, grouped by the original TurnId. Return actual saved complete
and partial records with their original response indices and the recorded
terminal outcome/reason. Do not insert synthetic active/completed executions,
output sources, queues, ToolCalls or delegation scopes. Use the existing
history-read coordination and Fault paths; do not introduce a recovery service,
new public RPC, availability state or retention cache.

The current [query DTO](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/query.rs) hard-requires
`TurnResultData.info: TurnStateData` and original `config`. Existing
`TurnFinished` does **not** save LoadId, original config/revisions, attempt
counts or complete Fault/cleanup metadata. `TurnIdentity.load_id` is already
optional. A minimal deliberate DTO distinction/optional retained metadata is
therefore preferable to filling these fields with the newly loaded Session's
values, current revision, zero counts or a fabricated full Fault. Preserve the
existing complete same-run result; represent unavailable historical metadata
honestly. The response still needs current Host identity/incarnation, the real
Session/Turn association and `data` as required by §9.

If a required result field truly cannot be represented without new durable
metadata, extend only the necessary terminal record format, explicitly version
and validate it, and retain truthful limited reads for supported old records.
Do not rewrite old logs or backfill invented metadata. A saved native summary or
compaction marker has only its real recorded coverage; do not invent a per-Turn
summary or terminal result for an unfinished log. This is a repair constraint,
not approval of a particular DTO/storage design.

The neighboring boundaries are intentionally narrower:

- §8 `turn.output.read` promises the Turn's **currently actually available**
  body projection. §9 lines 1728-1738 forbid rebuilding released source memory
  and old tools. Existing cold history reads can remain independent of old
  output/ToolCall state.
- `turn.get`/`turn.list` describe execution metadata/completion order, while §9
  forbids restoring execution. Their current retained-only behavior is recorded
  in the matrix, not separately declared a new cross-restart recovery defect.
  A saved result may expose its genuine recorded facts without reconstructing
  the complete old TurnInfo or expanding list semantics.
- `turn.wait` waiting on current retained execution is not permission to start
  an old Turn. The repair must settle how a saved terminal result is returned
  without waiting on nonexistent active work, but this is not another owner
  or a separate historical execution platform.

## Cross-cutting contracts outside a route count

### Initialization, trust and deadlines

`initialize` is outside the 52 business methods. In-process access is trusted;
stdio is the trusted local channel; WSS is opt-in TLS with actual configured key
validation before a connection receives read/operation authority. Invalid JSON
claims do not mint `CallerContext`; ordinary pre-init requests and RPC
notifications do not dispatch business mutations. First initialized eligible
connection gets operation rights, observers do not auto-promote, clones share
one connection, and a new connection competes afresh after release. See
[control.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/control.rs),
[host/network.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-core/src/host/network.rs), RPC and WSS code;
focused evidence includes:

- `TT:adapters.rs::malformed_initialization_and_json_claims_never_acquire_authority`
- `TT:adapters.rs::rpc_protocol_errors_and_notifications_do_not_dispatch_messages`
- `TT:wss.rs::first_initialized_connection_operates_and_observers_do_not_promote`
- `TT:wss.rs::old_network_request_blocked_before_acceptance_cannot_borrow_new_key_authority`

[deadline.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-transport/src/deadline.rs) starts one monotonic call
budget, default **30,000 ms**, covering local/RPC work through the complete
response; a positive override replaces that one budget. TCP/TLS/WebSocket/auth
initialization cannot restart it. Timeout ends the caller's wait, not accepted
business work, and never resends. Cancellation/timeout/connection/protocol/Fault
provenance remains distinct. Subscription setup uses that budget; its established
stream and the underlying Turn do not inherit a 30-second lifetime. Exact tests:

- `TT:wss_wait.rs::initialization_budget_covers_tls_websocket_and_authenticated_response_once`
  uses actual I/O, paused monotonic time and an exact 200 ms assertion, not an
  unstable wall-clock handshake deadline (task085).
- `TT:adapters.rs::timeout_and_dropped_wait_do_not_cancel_or_resend_accepted_work`
- `TT:observations.rs::established_stream_has_no_thirty_second_deadline`

### Acceptance, error and size boundaries

The §6/§10 typed request/receipt path carries the same business result as RPC.
Malformed RPC/request shapes are protocol errors; known methods with missing
capability and owner failures use the §11 Fault path. Unknown business Fault
fields are preserved rather than misclassified as a broken connection, while
partial creation and accepted `start_error` remain results containing their
real accepted facts. See
`TT:wait_budget.rs::unrecognized_business_fault_preserves_facts_without_failing_another_call`
and the matrix's partial-create tests. Transport/application code does not add
an arbitrary body/frame quota; the large-message assertion in
`TT:adapters.rs::os_pipe_frames_have_no_added_application_size_quota_and_shutdown_is_receipt_only`
is a regression check, not a capacity or unlimited-memory guarantee.

### Queries, output and retained-source lifetime

Current chapter-9 state/content queries carry current Host identity/incarnation,
actual target identity and `data`; Turn details/result use the original bound
config and terminal revisions, not a later Session config/revision. Same-run
unload/reload is tested, **not equivalent to Host restart** (G1). Cold history
coordination and cold `session.context` null/known distinction are implemented,
not old pending findings.

Agent/Session auxiliary list pages are intentionally `{items,next_cursor}`:
SPEC line 1328 and chapter 8 allow concrete simple pages; line 1469 concerns
chapter-9 state/content queries, not a universal wrapper for all auxiliaries.
Task073 independently reviewed this boundary. Host facilities and summaries
come from direct owners and omit large Role/body content; this is not an
allocation/performance measurement.

SO and [transport output.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-transport/src/output.rs) preserve
original Turn/response/ToolCall/channel sources and UTF-8 byte intervals. A
fixed read endpoint and real watermark bound gap filling; completion is not
proof that late tail data has arrived. The buffer merges matching overlaps
without rebasing or inventing bytes. A true empty tail differs from released
source, missing object or invalid cursor. Observers and receipt indexes do not
extend memory source lifetime. In addition to the matrix tests:

- `TT:observation_order.rs::actual_utf8_gap_reads_merge_late_overlaps_and_completion_before_tail`
- `CT:scoped_observation.rs::retained_subscription_does_not_retain_reclaimed_source`
- `TT:observation_tools.rs::observed_and_unobserved_runs_save_identical_history_and_cold_rpc_reads_do_not_create_runtime`

Workspace queries capture the requested Session's committed workspace, while
Session/Turn edit queries read original retained tool edits rather than Git
heuristics. Actual write/edit bytes and a later workspace-config change are
checked in
`TT:workspace_tool_changes.rs::actual_edits_remain_bound_to_original_tool_while_workspace_config_changes`.
`TT:workspace_tool_changes.rs::reopened_sessions_expose_missing_edit_coverage_but_current_turn_edits_are_queryable`
checks that restart does not pretend old edit coverage is an empty complete
set, while new actual Turn edits remain queryable.

### Persistence, partial acceptance and cleanup

The allowed parent/child modes are P/P, P/M and M/M; M/P is rejected. Memory
means no entity records, not discarded same-run context. Persistent startup
loads metadata/explicit close facts; history reads and load/resume validate real
JSONL, do not replay old inputs or repair corrupt bytes silently, and do not
recreate memory entities. A persistent Agent's memory default is runtime-only.

Accepted work belongs to its owner even if caller wait, connection or observation
ends. Partial Agent/Session/message creation is returned as real individual
steps. Metadata failures preserve the committed old config and expose the
original accepted save's fault; history/cleanup faults do not overwrite an
already claimed business outcome. Root directory ownership is retained through
required saves and actual worker/process cleanup during controlled graceful or
force shutdown. The accepted shutdown's owner lifetime is tested; a last-Host-Arc
self-cycle and a new unpublished-creation-save recovery entry remain explicitly
out of scope, as requested. No hidden retry or new recovery API is proposed.

Relevant exact evidence includes the matrix's partial/create/update tests plus:

- `CT:persistent_owners.rs::corrupt_history_blocks_only_that_session_and_preserves_the_file`
- `CT:persistent_owners.rs::close_persists_default_clear_before_closed_and_survives_restart`
- `CT:host_shutdown.rs::shutdown_force_failed_history_holds_root_until_original_save_continues`
- `CT:task033_tool_cleanup.rs::cancelled_runtime_future_is_not_actual_tool_completion`

### Reload and frozen binding

HR prepares resources outside owner locks, rechecks acceptance/publication
conditions, and publishes a committed generation atomically. Failed preparation
and candidates requiring restart do not partially apply unrelated fields.
Actual WSS key-value rotation invalidates old network connections and their
observations; old pending requests cannot borrow replacement authority.
Already accepted work continues with its original resources. Same key value via
a different environment reference does not revoke clients. Local/stdio channels
survive; existing observers do not inherit the vacant operator role. The next
Turn binds the new generation while the current Turn and internal continuation
retain old model/tool/prompt/approval config. This is exercised by
`TT:reload_network.rs::canonical_wss_rotation_keeps_an_accepted_http_turn_and_next_turn_binds_new_resources`
with two real local HTTP providers and TLS clients, not live-provider validation.

### Independent delegation, ordinary tools and approval

SPEC §13's owner-bound `agent` tool is an internal capability adapter, not an
external connection or authority shortcut. [scope.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-tools/src/scope.rs),
[core_adapter.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-tools/src/core_adapter.rs) and owner collaboration
code register actual delegation before send, preserve the original receiver,
keep child work independent of the parent's Turn/wait, and deliver one actual
completion input through the ordinary two-queue rules. Launch ToolResult,
child outcome, notification acceptance, parent consumption/save and memory
retirement remain separate facts. Observer delivery is not the result-handoff
owner. Exact evidence includes:

- `CT:delegation_owners.rs::child_handoff_survives_parent_cancel_and_notifies_paused_original_parent_once`
- `CT:delegation_owners.rs::wait_ignores_next_turn_and_paused_next_step_without_consuming`
- `crates/host-tools/tests/host_integration.rs::real_host_delegation_survives_parent_turn_and_client_disconnect`
- `crates/host-tools/tests/host_integration.rs::real_agent_delegation_copies_config_and_reclaims_temporary_owners`

The production registry contains `agent`, `read`, `write`, `edit`, `bash`.
`apply_patch` is absent and not a SPEC requirement. Local tools use their actual
bound workspace/resources and source identity, not a global current-directory
switch; this is not an OS sandbox. [approval.rs](https://github.com/zqcli/minicore-host/blob/2615fe00ff385a74c6ef6a302c169ccc2b6b4410/crates/host-resources/src/approval.rs)
freezes Ask (default), Auto or ReadOnly per Turn, with exemptions only for known
builtin read/inspection/wait origins. Registered external tools named `read` or
`agent` do not acquire those exemptions. Original interaction/cancel/deadline
owners decide allow-once/deny and prevent late tool execution. Exact evidence:

- `crates/host-app/tests/tool_approval.rs::binary_omitted_approval_is_ask_and_allow_once_does_not_approve_the_next_call`
- `crates/host-app/tests/tool_approval.rs::binary_read_only_blocks_local_and_agent_mutations_but_reads_inspects_and_waits`
- `crates/host-app/tests/tool_approval.rs::binary_policy_update_is_frozen_through_internal_loops_and_applies_next_turn`
- `crates/host-resources/tests/tool_approval.rs::external_factory_named_read_keeps_mutating_semantics_in_every_mode`

Pinned Runtime executes tools within one model batch serially. SPEC permits
parallel execution rather than requiring it; this is not a missing mandatory
parallel-batch feature. Real independent Sessions are exercised concurrently,
including `CT:runtime_owners.rs::slow_session_does_not_block_another_session_or_receipts`.
No performance conclusion follows from concurrency tests.

## Preserved review evidence and remaining limits

| Record | Usable scope, not an extended acceptance claim |
| --- | --- |
| [073](../tasks/073-owner-discovery-review/TASK.md) | Independent owner-discovery approval at `f0900a7`; reported 486 scoped cases/one doctest and seven probes. Concrete list-page interpretation remains applicable; no full product/performance approval. |
| [076](../tasks/076-diagnostics-review/TASK.md) | Preserved transport/provider/shared-health review scopes only. Earlier raw artifacts were lost; the record does not independently validate later core diagnostics. |
| [078](../tasks/078-approval-review/TASK.md) | Preserved production approval review, exact builtin-origin semantics and actual-binary tests. Lost independent probe artifacts are not freshly replayed by this audit. |
| [081](../tasks/081-query-pipe-teardown/TASK.md) | Actual pipe BrokenPipe diagnosis and request-half-close fixture correction; not a production query repair or restart-result test. |
| [083](../tasks/083-second-workspace-recovery/TASK.md) | Source/evidence replacement limits. Missing unpublished artifacts are not recovered or fabricated. |
| [084](../tasks/084-app-log-fixture/TASK.md) | Actual binary readiness selector disambiguates component plus event; 26 app cases at its fixed source. Neither startup diagnostic was removed to satisfy the fixture. |
| [085](../tasks/085-wss-deadline-fixture/TASK.md) | Exact monotonic initialization-budget fixture and 143 scoped transport cases at its fixed source; not a production deadline increase. |

No new P0 is established by this audit. **G1/P1 blocks saved-result acceptance.**
The already identified §12 actual retry-wait elapsed field is missing in pinned
Runtime; a separate worker owns its minimal implementation, so this audit does
not duplicate that assignment or assume its later fix. Full business diagnostics,
new performance/capacity evidence and external final review remain separately
unverified here. Do not carry forward old pending notes for cold history,
context, retained query envelopes/config/revision or discovery that are now
implemented; equally, do not turn the full gate into acceptance of untested
saved-result behavior.

Code-reference policy remains unchanged: minicore-agent
`f36a87366d8e2fe0a1c8216463887902e471c739` is a source of suitable mature components,
not a compatibility/behavior baseline. TUI
`3beeb1d8a525e58ce6352e05061f242c5580e248` is unchanged/deferred; both reference
projects still lock their own Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`.
No TUI adaptation, joint compatibility, live-model run, remote update or final
release approval is included in this document.
