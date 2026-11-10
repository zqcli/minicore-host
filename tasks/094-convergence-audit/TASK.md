# Task 094: fixed-source convergence and remaining-scope audit

## Scope and conclusion

Read-only production audit at Host
`50a1b2fedae73ce5c8b0a461a5bd6ed0edce68c8`, tree
`b616d753d0d35622ccefa10dc7752b7b2df21c46`. Branch/worktree:
`task/094-convergence-audit`, `minicore-host-task094`. Only this document changes.
No new production defect was established by this bounded source/evidence check.
This is not complete first-version acceptance or an independent final review.

Read AGENTS, SPEC, [the fixed historical contract audit](../../docs/current-contract-audit.md),
PROJECT/handoff records, relevant production implementations and named tests.
The historical audit describes `587318b`; its G1 and retry-wait omissions must
not be presented as current open findings. The current source contains the
saved-result/P2 validation and original-compaction-Fault fixes described below.

- SPEC v4.93 is unchanged: SHA-256
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Current Cargo manifests/lock pin Runtime 0.10.0 at
  `3aa47629929906832dcb309afb7d242c1ae3e53f`, including measured retry waits.
  The older review checkpoints below used their stated dependencies.
- Requested worker configuration: `gpt-6-astra` / `max`; effective runtime
  configuration is not independently attested.
- Minicore Agent is code reference only, not a compatibility baseline. TUI
  remains unchanged/deferred at `3beeb1d8a525e58ce6352e05061f242c5580e248`.

## Evidence used, with its real scope

**G: exact-source composed gate.** The coordinator ran `scripts/check.sh` with
offline Cargo at `50a1b2f`, 2026-11-10 03:22:12–03:26:02 UTC, and reported exit 0.
I read the retained output, rehashed it and its inventory, and checked named
assertions below. It includes workspace/all-target strict Clippy, tests, strict
rustdoc, formatting and whitespace; 937 unique Rust cases, 4 doctests and 20
Python cases. Recounting the list by executable plus test name gives 124
nonempty executables and the same 937 cases. Counts describe this gate, not
acceptance coverage, capacity or a completion percentage. I did not rerun it.

**R91: saved-result scoped independent APPROVE.** Report commit
`e39c1c87132e0c0d721c3637e18d6770ae8ed322`,
`tasks/091-saved-result-review/TASK.md`, accepts exact source
`7b6f14080a1fa1d3ac0dbe8623dd59f641dbdadf`. The report and unchanged-probe green
log were read/rehashed. It closes the real restart-result P1 and reproduced
invalid retained/saved wire-combination P2. Its independent review is not
silently extended to subsequent diagnostics or Runtime changes.

**R92: compaction scoped independent APPROVE.** Report commit
`e3228f9e60a84681531a259e8af633cf30e3e728`,
`tasks/092-compaction-review/TASK.md`, accepts
`a07fd1d60c17cda7d5a56b7083e4d6e6259e6d16`. The reviewed production change is
integrated at `1791455`; query-fixture adaptation is `50a1b2f`. Read the report
and rehashed its fixed gate. Independent loopback/provider, real-inode failure
and narrow binding probes supplement the producer's tests, not a live model.

**R89/P90:** [task089](../089-session-model-diagnostics-review/TASK.md) records
scoped scheduling/Closed/model diagnostics and elapsed-wait consumer review;
[the publication record](../../docs/runtime-retry-wait-publication.json) ties
the implemented Runtime source to the current pin. Task090 report commit
`9838865991b2dc3d89571923721e97a84120d058` contains
`docs/performance-f36761a.md`: fresh controlled local measurements at `f36761a`,
not a final-tree benchmark, live-provider run or TUI display measurement.

## Six stages: implemented behavior versus remaining work

### 1. Core contracts and storage

**Implemented and verified:** typed identities/receipts/Faults and validated
native JSONL; real file ownership, append/sync confirmation and original-save
continuation, rather than accepting readable bytes as persisted. Current
`host-storage/{strict_json,journal,file_handle}.rs` and Session history ownership
were sampled. G includes real corrupt-file, partial-creation and failed-save
tests, notably `persistent_owners::corrupt_history_blocks_only_that_session_and_preserves_the_file`
and `host_shutdown::shutdown_force_failed_history_holds_root_until_original_save_continues`.
R91 plus G verify saved result reads and valid DTO branches without restoring
execution. A saved result does not manufacture old config, LoadId or counters.

**Remaining / unverified:** no new mandatory storage gap identified here. The
final diagnostics integration still needs its composed gate/review. Controlled
filesystem/save failures are not proof of every OS/filesystem crash-durability
case; no migration, automatic corrupt-log repair or unpublished-creation
recovery platform is required or proposed.

### 2. Host and entities

**Implemented and verified:** direct Host → Agent → Session ownership, Role,
permitted persistent/memory combinations, explicit/default targets, immutable
Turn configuration, partial creation, durable close, explicit deletion and
graceful/force shutdown barriers. Source checks included shutdown's actual
child/save/resource retirement and whole-candidate configuration publication.
G includes `persistent_owners::default_session_real_file_failure_keeps_published_agent_receipt`,
`close_persists_default_clear_before_closed_and_survives_restart`, and
`host_shutdown::force_keeps_owned_save_and_directory_after_callers_and_host_handle_drop`.
Process tests exercise real startup, root exclusivity, protocol isolation and
shared logging health; initialization/receipts are not mistaken for completion.

**Remaining / unverified:** final task053 diagnostics handoff and its independent
review/integration are outside this fixed tree. Many owner/Session diagnostics
are already present and pass G; this is not a claim they are all absent. No
additional entity or parallel ownership framework is justified by this audit.

### 3. Session and Runtime

**Implemented and verified:** two queues, same-Turn internal continuations,
actual model/tool execution, frozen approval, owner-bound agent six actions,
independent child execution/result handoff, cancellation, manual/automatic
compaction and save/cleanup barriers. `take_batch`, report claim, native history
acceptance and compaction Fault handoff were inspected, not just their signatures.
G includes `runtime_owners::initial_batch_uses_two_independent_queues_once`,
`host-tools/host_integration::real_host_delegation_survives_parent_turn_and_client_disconnect`,
actual-binary file/process/approval cases, and
`session::tests::compaction::compaction_original_failure_survives_cancel_and_independent_save_failure_once`.
R92 verifies original utility Fault 3702, separate persistence failure 3301,
cancel-first precedence, exact automatic binding and one-shot handoff. R89 and
the current Runtime pin cover actual retry-wait elapsed facts, not planned delay.

**Remaining / unverified:** real external-provider execution has not been done;
scripted/gated HTTP and real Runtime are not that evidence. Final task053
diagnostics composition/review remains separate. No speculative retry,
recovery, quota or old-Agent tool compatibility layer is needed. Ordinary tool
batch parallelism is permitted, not mandated; separate Sessions really run
independently. See the tool/context distinctions below.

### 4. Client access

**Implemented and verified:** common typed business handlers serve in-process,
OS-pipe stdio and authenticated TLS/WSS. Separate initialization, current
operation authority, one whole-call deadline, accepted-work survival, scoped
observations and UTF-8 gap reads have behavioral evidence. Source route checks
agree with the SPEC catalogue; subscription has real special handling rather
than a stub. G includes `reload_network::canonical_wss_rotation_keeps_an_accepted_http_turn_and_next_turn_binds_new_resources`,
`interaction_answers::real_answer_survives_operator_replacement_preserves_pause_and_retains_runtime_result`,
`observation_order::actual_utf8_gap_reads_merge_late_overlaps_and_completion_before_tail`,
and `task087_saved_result_contract::saved_result_and_wait_share_exact_contract_on_typed_stdio_and_wss`.
R91 additionally rejects malformed result/wait replies through the stdio client.

**Remaining / unverified:** actual TUI use and live/WAN deployment are not
covered by these local transports. No missing named resource route was found;
route existence does not establish all acceptance behavior. No Host-to-Host
RPC or remote-subagent platform is part of this first-version scope.

### 5. Actual TUI integration

**Implemented and verified:** Host client APIs and real transport fixtures
exist; this does not implement this stage. The TUI reference is unchanged.

**Explicitly deferred / not done:** TUI-originated multi-turn input, create/switch,
tools, streaming/gap recovery display, history, configuration and controls
against this Host/Runtime. SPEC §13 explicitly requires actual TUI integration
for full acceptance. The current instruction not to change TUI is respected;
neither the transport tests nor the reference TUI's different Runtime pin can
be counted as that acceptance.

### 6. Convergence and cleanup

**Implemented and verified:** fixed-source behavior gates and scoped independent
reviews now cover the previously demonstrated saved-result and compaction
fault defects. G is complete for this exact combined source, not still pending.
P90 supplies fresh representative local inventory/queue/concurrent execution/
observation/process-cleanup measurements with artificial waits identified.
The public route set contains only the SPEC methods, not obsolete
`input.push`, `input.resume` or `session.open` aliases. No concrete remaining
replacement-code removal was established by this bounded read.

**Remaining / unverified:** finish task053 and its scoped review, validate the
resulting composed tree, refresh the coordinator-owned summary/handoff, and
obtain independent final review. P90 predates later changes; it is not a final
benchmark or saturation/large-corpus/long-soak result. Live-provider and TUI
acceptance remain separate outstanding work, not hidden by green tests. No
arbitrary stress matrix or compatibility rewrite is requested by this audit.

## Specific scope questions and corrected stale findings

- **All 52 methods:** compared the §8 table with `host-protocol/src/catalog.rs`
  and `host-transport/src/operations.rs`: identical sets, with 51 ordinary
  handlers plus actual `events.subscribe` paths in `client.rs`/`rpc.rs`.
  Read the historical per-method matrix and sampled the substantive owner
  paths and assertions above. This establishes routing and focused behavior,
  not completion by method count. Auxiliary workspace/discovery/model/tool
  queries retain their deliberately basic scope; no generic query platform is
  required. Existing real filesystem/diff/query tests are included in G.
- **Saved result:** the old audit G1 is closed, not merely hidden by history
  reads. Current `session/observation/query/saved.rs` validates actual saved
  terminal facts; retained active/finalizing ownership wins before/after I/O.
  `query.rs` permits only retained info/config or saved terminal metadata, and
  associates LoadId only with retained execution. `turn.wait` checks again after
  subscribing. R91 and G cover restart, corruption, saved/retained shape and
  all three client paths. Old output buffers/tools/queues are not reconstructed.
- **Original compaction Fault:** `ActiveTurn` consumes only the exact automatic
  Turn/Load/Loop/request binding. The real direct-owner Fault survives the
  Runtime Prompt wrapper; cancellation or a prior primary is not replaced, and
  original save failure stays independent. It is not logging-only suppression.
  Current real compaction diagnostic tests cover no/closed logger, original
  receipts, pending versus confirmed save, reentrant Subscriber callbacks and
  body exclusion. The old 3702→4999 pending note is superseded by R92 and G.
- **apply_patch:** the immutable SPEC has no named `apply_patch` requirement.
  Actual `HostResourceBuilder::builtins` registers `agent` and the shared local
  specs `read`, `edit`, `write`, `bash`; `tool.list` uses that registry. The
  existing absence assertion passes in G. `apply_patch` is not implemented or
  advertised; edit/write are not called patch compatibility. Its presence in
  reference Agent does not make it an unfulfilled Host requirement. A future
  explicit requirement would need its own implementation and real file tests.
- **Cold context:** no violation established. §8 requires basic truthful context
  facts, not an implicit load or invented token estimate. `session/context.rs`
  reads only current owner state: unknown cold history is `null`, a validated
  empty history has real zero item counts, and unload can retain known content
  without claiming loaded execution. Latest complete-response usage preserves
  absent provider fields; it is not current effective prompt size. The dedicated
  `session_context` tests for cold/load/unload, unknown/real-zero usage and
  corruption pass in G. No new polling/cache or eager file read is needed.

## Audit validation and handoff

No Cargo/build/full-suite run was started by this worker. A private static
script checked the fixed source/SPEC, route sets, builtin registration, log
hashes, unique inventory and selected named passing assertions. Its first regex
omitted the two `C，可带 D` creation rows; that was a corrected audit-script
fixture error, not a production missing-method finding.

Reused artifacts below were actually rehashed. G files are under the
coordinator's `recovery-second/validation`; review logs are in their documented
review evidence directories, not new executions by this audit.

| Artifact | SHA-256 |
| --- | --- |
| `full-50a1b2f.log` | `e09269d6cd3db15f1b93a80df03e851d6b9a23ce327fd1fa05943d165f48f739` |
| `full-50a1b2f-inventory.log` | `ba5489aa5f8a53a6ca2d1472d02f3c934cf3a526925ad281c38cf246ab85a568` |
| R91 `fixed-independent-green.log` | `ea13ad112423e54d186c1ef139421a6a79615db562a976759cac5d273461b96e` |
| R92 `fixed-gate.log` | `db9c114dda504d862d777798e80403fe37228b6326588ef49c6844cbfd9bf49e` |
| Task094 private `static_check.py` | `c9b0c7fbe991b9d89c48923bb31b13e50cce5a8385e6a2e3d27a8607d622afa5` |
| Task094 private `static-check.json` | `282345fd7c6502bdbd1a251a55fb2ff2fd8e3f72d97fd9d1769a4750a1ac6cf9` |

Final documentation checks: relative links resolve, `git diff --check` and
`scripts/check-whitespace.sh` pass; the staged change is only this TASK.md.
No production, dependency, SPEC, PROJECT, README, TUI, main or remote action.
The coordinator owns integration, later-source gates and publication.
