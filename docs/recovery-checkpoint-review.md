# Fixed recovery checkpoint

## Current review target

Review published `dev` [53167d3117323d2b126c453fbb53fa980bdb72ee](https://github.com/zqcli/minicore-host/commit/53167d3117323d2b126c453fbb53fa980bdb72ee), compared with preserved `ccadc3013d084a47a3a9cfcd48fcfffdeca34253`. It exactly matches source `50a1b2fedae73ce5c8b0a461a5bd6ed0edce68c8` and tree `b616d753d0d35622ccefa10dc7752b7b2df21c46`. The [99-entry mapping](recovery-publication.json) preserves all 382 commits, original raw messages and actual zqcli author/committer facts. Main is unchanged. This is a backend checkpoint, not authorization to merge or a complete first-version claim.

A fresh whole-workspace gate passes **937 unique Rust tests, 4 doctests and 20 Python checks**, strict all-target Clippy/rustdoc, fmt and committed whitespace. [The manifest](checkpoint-50a1b2f-validation.json) contains exact commands and committed raw gate/inventory logs. Exact-head [CI42749273803](https://github.com/zqcli/minicore-host/actions/runs/42749273803) passed; previous ccadc CI42748451681 also passed. The older 39b CI's App Option-fixture compile failure is fixed by 0cc9e61, not erased from the history.

Current corrections and review entry points:

1. `session/observation/query/saved.rs` and result DTOs: actual saved result projection without old execution restoration or fabricated metadata. [Task091](../tasks/091-saved-result-review/TASK.md) approves 7b6f140 after a real restart P1 and bidirectional wire P2 were fixed.
2. `session/compaction.rs`, Session's private automatic binding/Fault handoff, and `session/history.rs`: preserve the original 3702 through Runtime Prompt, retain separate 3301 persistence failure, maintain cancellation/cleanup barriers. [Task092](../tasks/092-compaction-review/TASK.md) approves source a07fd1d; this gate tests the composed normal published Runtime pin and query API.
3. [Task089](../tasks/089-session-model-diagnostics-review/TASK.md): actual scheduling/cancel/Closed and report-derived attempt/usage/retry-wait diagnostics. The Runtime source 2f7aeb9 is public as 3aa47629929906832dcb309afb7d242c1ae3e53f; [its publication evidence](runtime-retry-wait-publication.json) and CI42746444546 confirm success. No new retry event field, inferred timing, local dependency override or policy change.
4. [Task090](../tasks/090-fresh-performance/TASK.md) and [the report](performance-f36761a.md): three measured real-process runs plus a separate smoke run, exact commands and committed raw JSONL. Production source is f36761a, not the later diagnostic composition. No live model, TUI, saturation or capacity claim.

Remaining 053 force/preparation/internal-message/failure-scheduling diagnostics and their independent review are not in this fixed source. Full-product external review, real authorized external-provider and actual TUI acceptance remain unperformed; TUI is explicitly deferred this round. [PROJECT.md](../PROJECT.md) provides the six-stage boundaries and explicit local-use limits. New scoped approvals and a passing full gate do not substitute for those open items.

Earlier sections below are immutable-in-meaning checkpoint observations. Their then-open saved-result/compaction/timing findings are superseded by the fixed corrections above; their historic counts and dependency pins remain attached to the old sources.

## Previous checkpoint cc2bf0f (historical status)

Review fixed `dev` commit [cc2bf0fbd40f04920d611332ef0ff55adc724aec](https://github.com/zqcli/minicore-host/commit/cc2bf0fbd40f04920d611332ef0ff55adc724aec), compared with `2615fe00ff385a74c6ef6a302c169ccc2b6b4410`.
It exactly preserves source `d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06` and tree
`34266ef4a0566e3695fe3e3eaa0aad92ef8fb7d2`. The [80-entry mapping](recovery-publication.json)
retains the previous 75 entries, original messages and parent order. All 363 remote
commits and unchanged README-only main are preserved; author/committer is verified
as zqcli, not inferred from the local author. Exact-head
[CI42743579503](https://github.com/zqcli/minicore-host/actions/runs/42743579503) passed.

The five-commit backup adds original model attempt/usage facts, owner reclamation,
original Session metadata operation retention and the previous verified handoff.
Source through d9ce4e9 passed 20 focused combined cases and strict core/transport
all-target lint. Reclamation and Session metadata have scoped independent
approvals in tasks082/080; the preceding full local 908/4/20 gate below remains
attached to source 587318b rather than asserted for each later commit.

**Open P1:** real saved persistent `turn.result` still fails after controlled
restart in this checkpoint. [The 52-method audit](current-contract-audit.md) records
the actual failing probe and per-method boundaries, not an acceptance percentage.
Task087 is implementing and validating a read-only projection of actual saved
facts; unavailable old execution metadata must remain absent. Later local
scheduling/Closed diagnostics at f36761a, compaction/result-handoff diagnostics,
Runtime elapsed-time integration and fresh performance evidence are separate
work. Runtime 2f7aeb9 received scoped independent approval but has not been pushed
or pinned; no public event timing expansion is claimed. TUI/live-provider and
external final acceptance remain unperformed. No main merge is authorized here.

Earlier checkpoint descriptions below are preserved historical observations;
the current status above supersedes their then-pending review/CI descriptions.

## Later verified checkpoint

The next five-commit backup is published as
[`2615fe00ff385a74c6ef6a302c169ccc2b6b4410`](https://github.com/zqcli/minicore-host/commit/2615fe00ff385a74c6ef6a302c169ccc2b6b4410),
identical to source `587318b69a2e1f72d0f82999d3f6f6badacfe0e4`, tree
`03aad0d4af62a0b89fadcb3d3dc78d63d12425b9`. Compare with the preserved
9f75 checkpoint below. All 358 published commits remain, and the cumulative
[publication map](recovery-publication.json) has 75 entries. Main is unchanged.

This adds Session history/metadata failure facts, Agent metadata/reconcile
facts, the preceding checkpoint documentation and the deterministic real-WSS
initialization test. The new [full-gate manifest](checkpoint-587318b-validation.json)
records **908 unique Rust cases, 4 doctests and 20 Python checks**, with strict
lint/docs, fmt and whitespace. Exact-head
[CI 42741046081](https://github.com/zqcli/minicore-host/actions/runs/42741046081)
passed. The previous 9f75 CI also completed successfully after its earlier
in-progress observation below.

Scoped independent review now approves the tool/process slice at source 101abcb,
the Agent metadata slice at19e7bbb, and the WSS test-only correction atf931bcc
(integrated587318b). See [task080](../tasks/080-session-diagnostics-review/TASK.md)
and [task082](../tasks/082-owner-diagnostics-review/TASK.md) for exact boundaries.
This does not approve the newer model, reclamation or original Session metadata
operation changes at local `d9ce4e9727204e0727943dc83d366dba9142604d`.

The Runtime actual retry-wait measurement is separately implemented locally at
`2f7aeb9a2112e69cf1918c375916656e54d1208e` and is under independent review and
isolated Host integration. It is not in this checkpoint's published Runtime pin.
Final contract audit, fresh performance evidence, live-provider/TUI acceptance
and final external review remain uncompleted. The earlier evidence below is
preserved as the state of that specific earlier checkpoint, not current status.

The fresh contract audit has since reproduced a P1 at this fixed source:
`turn.result` returns3201 after a controlled persistent restart despite a saved
TurnId, terminal outcome and exact result records. Cold history still works and
does not restore execution. SPEC section8 requires held or saved result reads;
the existing query only examines the current in-process completed map. Its
correction must not fabricate unavailable historical LoadId, configuration,
revisions or counters. The actual restart probe is retained by task086 and is
not one of the passing908 cases. The checkpoint is not ready for final approval.

## Source and publication

- Review `dev` at [`9f75d0e770b39b99614530fa126fd90952f95fe7`](https://github.com/zqcli/minicore-host/commit/9f75d0e770b39b99614530fa126fd90952f95fe7), not an advancing branch head.
- Compare with [`8c5fe493f9b9e78e15ef831c532bc267d0c7fb53`](https://github.com/zqcli/minicore-host/compare/8c5fe493f9b9e78e15ef831c532bc267d0c7fb53...9f75d0e770b39b99614530fa126fd90952f95fe7).
- Verified local source: `101abcb25a416a81f8350ca1acdbe6078b0de0bc`.
- Identical tree: `16cb1a24ea981d1fb4bd43c341ffede2567634c8`.
- The [70-entry publication map](recovery-publication.json) preserves every
  previous mapping and records the three recovery backups. The [earlier recovered
  mapping evidence](recovery-publication-8c5fe49.json) is preserved separately. API, ls-remote and
  actual fetch verification agree. The 353-commit published history is intact;
  main remains `b341b163e14543bfbe7f8d392ea243b4a123cdf7`.

API reconstruction preserves file bytes, modes, trees, messages and parent order,
but changes commit identity and timestamps. Actual remote author/committer is
zqcli; differing local Codex authors are recorded separately, not relabelled.
No Co-authored-by trailers, force updates or main changes were made.

## Scope and decisions

The checkpoint reconstructs missing unpublished diagnostics after the second
workspace replacement, on top of the already published executable backend.
It is not a recovered copy of the unavailable old local Git objects.

- `crates/host-core/src/session*`: accepted/bound input facts, primary terminal
  claims and independent save failures, close acceptance versus actual closure,
  original tool completion and process-wait summaries. Records contain selected
  scalar facts and are emitted outside owner locks; they neither supply history
  nor determine successful business completion.
- `crates/host-core/src/{host,agent}*`: initial entity startup/create/shutdown
  facts. Unused Span destruction is kept outside admission guards. Immutable
  Host mode prevents a persistent instance being reported as memory during
  stopped/repeated shutdown or store release.
- `crates/host-transport/tests/task070_query_recovery.rs`: the real pipe fixture
  drains the server after request EOF before dropping the response reader.
  Strict transport errors remain intact; controlled probes preserve both valid
  teardown orderings.
- `crates/host-app/tests`: process readiness selects both `host.process` and
  `host.started`, not the earlier owner event with the same name. This changes
  only the fixture and adds a real-binary regression retaining both records.
- Existing production credential exclusion, tool approval, health, provider and
  transport diagnostics remain present. The synthetic-credential binary test
  passes, including actual tool output and durable history checks. This is not
  OS isolation from other processes running as the same user.

SPEC remains unchanged at SHA-256
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Runtime is the sole pinned Git dependency `69faf9196991524b5549499d6208a1eaf567bf56`,
version 0.10.0, from `feat/host-history-handoff`; there is no local source patch.
Agent `f36a87366d8e2fe0a1c8216463887902e471c739` is code reference only. TUI
`3beeb1d8a525e58ce6352e05061f242c5580e248` is unchanged and still uses its own old
Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`. No joint TUI test is claimed.

## Fresh verification

At exactly source 101abcb, with official Rust 1.90.0 and the committed lock:

```sh
cargo test -p minicore-host --test credential_environment --test task084_app_log_fixture --locked --offline
CARGO_NET_OFFLINE=true ./scripts/check.sh
cargo test --workspace --all-targets --locked --offline -- --list
```

All commands exited 0. The complete gate includes 899 unique Rust cases,
4 doctests and 20 Python tests, plus formatting, committed/worktree whitespace,
strict all-target Clippy and strict rustdoc. Counts come from the separate list
command, not duplicated child-process output. The [validation manifest](recovery-checkpoint-validation.json)
records source, environment, actual commands and retained log hashes.

Exact-head [hosted CI 42739672016](https://github.com/zqcli/minicore-host/actions/runs/42739672016)
was still running at the last observation. This is not a CI success claim.
Previous ee9 CI failed at readiness `listen_port` extraction, before the credential
assertions; it did not demonstrate credential disclosure. Earlier 138 CI exposed
a separate 200 ms TLS fixture race, described below.

Fresh independent review approved Session accept/terminal/save/close scope at
source `87fa305fb77cae1e1c1c0b11e1f8481dc2e1455a` and initial Host/Agent scope at
`7781a1d8f58967dfb76d52fbef6a224f813911d6`. The latter retained its original
three reproductions, 71 scoped cases and eight supplementary checks. A public
race probe did not hit the narrow store-release window; that mode correction is
justified by the immutable constructor fact, not a fabricated dynamic red.
Independent review of later tool/process summaries is still in progress.
These are scoped internal reviews, not the requested final external review.

## Open boundaries and next checkpoint

- The WSS short-budget fixture can legitimately reach the absolute 200 ms
  deadline before its peer completes TLS. A deterministic red reproduced the
  exact TLS EOF after the client's correct WaitTimeout. Its replacement uses
  controlled time and real TCP/TLS/WebSocket/authentication, without enlarging
  the deadline or swallowing errors; it is not part of this fixed checkpoint.
- New Session history/metadata and Agent metadata/reconcile slices after
  101abcb have focused integration checks, but are not included in its full gate.
  Remaining owner reclamation, retry/use, scheduling/compaction and result-handoff
  diagnostics still require completion and scoped review.
- Actual retry-wait elapsed time is not exposed by the fixed Runtime. Do not
  substitute planned delay for measured elapsed time; record this remaining
  diagnostic difference rather than inventing a value.
- Complete the fixed-source method-contract audit and fresh performance runs
  with retained raw evidence. Earlier lost measurements cannot certify this tree.
- Live external-provider and actual TUI acceptance have not been run. TUI changes
  are intentionally deferred. No first-version completion or capacity claim is made.
- Continue five-commit progress backups while recording incomplete review state;
  perform final aggregate checks and external review before considering main.

Earlier checkpoints and the exact lost/recovered boundary remain in
[task 083](../tasks/083-second-workspace-recovery/TASK.md). No missing old artifacts
or test results are inferred merely from a recorded SHA.
