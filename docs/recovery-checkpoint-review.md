# Fixed recovery checkpoint

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
