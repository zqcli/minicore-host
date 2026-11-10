# Refresh the fixed-source first-version contract audit

## Scope and provenance

Base and audited production source:
`587318b69a2e1f72d0f82999d3f6f6badacfe0e4`.
Tree: `03aad0d4af62a0b89fadcb3d3dc78d63d12425b9`.
The coordinator reports equivalent remote dev
`2615fe00ff385a74c6ef6a302c169ccc2b6b4410`; this worker did not fetch, push or
modify main/remote refs. Topic: `task/086-final-contract-audit`; isolated worktree:
`/workspace/shared/minicore-host-task086`. Requested worker configuration:
`gpt-6-astra` / `max`; effective runtime configuration is not independently
attested.

Own only this task record and `docs/current-contract-audit.md`. The parent owns
PROJECT.md and final integration; historical `docs/method-coverage.md` remains
unchanged. No production, committed tests, manifests, lockfile, SPEC, workflow
or other shared-document edits. No recovery of the lost task075 audit artifacts
is claimed. This is a fresh audit of the fixed source, not later diagnostics,
Runtime work or later fixes.

SPEC v4.93 SHA-256 remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Runtime remains Git `69faf9196991524b5549499d6208a1eaf567bf56`, version `0.10.0`.
Minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739` is a code reference,
not a compatibility/behavior baseline. TUI
`3beeb1d8a525e58ce6352e05061f242c5580e248` is unchanged/deferred; the reference
Agent/TUI Runtime lock `b276d9c0a1f3a3c06e32029f831ccd100c9699c7` is not this
Host's Runtime baseline. No live-provider or TUI test is claimed.

## Deliverable and result

[The audit](../../docs/current-contract-audit.md) maps all 52 canonical methods to
actual shared routes, direct owners, exact focused test names, checked contracts
and retained/missing boundaries. It separately covers initialization/auth/operator
rights, one 30-second call budget, established subscriptions, UTF-8/source
lifetime, modes/partial creation/save ownership, bound configuration/query
shapes, reload key publication, DSH delegation and production approval.

The complete SPEC and relevant actual source/test content were inspected,
including tasks 073/076/078/081/083/084/085. Old pending notes for cold history,
context, same-run query corrections and discovery were not carried forward.
Concrete Agent/Session list pages are not wrongly required to have generic
chapter-9 envelopes. All 52 have routes; that does not establish full acceptance.

**One fresh P1, G1:** `turn.result` cannot read a valid saved terminal result
following actual controlled Host restart, even after explicit Session load.
Reported to the parent with exact SPEC/source references; the parent accepted
the finding. The audit recommends only a truthful read-only saved projection,
not old execution/cache restoration or a new recovery API. Missing original
config/revisions/attempts/Fault/Load metadata must not be fabricated from new
state. The known §12 actual retry-wait elapsed field already has a separate
implementation owner and is not a new duplicate assignment here.

No new P0 was established. Controlled shutdown must retain actual save/cleanup
ownership; the declined unpublished-creation-save recovery entry and last-Arc
self-cycle remain out of scope. Full business diagnostics, fresh performance,
external final review and later integration tests remain with their owners.

## Fresh targeted probe

Only one temporary, untracked Rust test was added for ambiguity resolution:
`crates/host-core/tests/task086_saved_result_probe.rs`, test
`persisted_turn_result_remains_queryable_without_restoring_execution`.
It was removed from the worktree after copying evidence; it is not in this
commit or counted as a passing committed regression.

The probe uses actual Host/Agent/Session owners, pinned Runtime, production
`OpenAiChatModel` over a real loopback HTTP/SSE response and a temporary persistent
root. It does not substitute a fake Runtime or fabricate a terminal file. It:

1. Creates persistent Agent/Session, sends one real input and waits for the
   completed result with `PersistenceState::Persisted`.
2. Runs graceful shutdown to stopped, verifies the actual JSONL's original
   TurnId/`Completed` terminal plus input/response (three records), and drops old
   connection/Host handles.
3. Reopens that root, checks stable HostId/new incarnation, reads saved answer
   through cold history, queries original result/list, explicitly loads, then
   repeats those result/list queries.
4. Requires no active execution and byte-identical JSONL; the final saved-result
   assertion is deliberately the contract being tested, not an expected error.

The observed output in both runs is:

```text
before restart: completed/persisted, valid saved terminal matches TurnId; actual JSONL records=3
cold history is readable; cold turn.result=Err(("SESSION_TURN_NOT_FOUND", 3201)); cold turn.list count=0
after explicit load turn.result=Err(("SESSION_TURN_NOT_FOUND", 3201)); turn.list count=0
```

Both runs exit **101**, one test failed at the final assertion:

```text
SPEC §8 turn.result requires the actual saved result; loading history must not restore execution, but must not discard durable result facts
```

The first compile/run took 50.57 s; its untracked probe had unused-import/module
warnings. Only those unused items and formatting were cleaned; the second run
compiled in 1.24 s without those warnings and failed the same assertion. This is
one unique reproduced failing case, not two different cases or a passing gate.

Fresh local evidence directory: `/workspace/shared/task086-audit-evidence/`.
This path is a local handoff, not an assertion that lost earlier artifacts were
recovered or that these artifacts are independently archived forever.

| Artifact | SHA-256 / outcome |
| --- | --- |
| `task086_saved_result_probe.rs` (clean second-run source) | `df20555271e12609a92358b658ea4519fbaa2d74b32d845baec05d0d7781d383` |
| `saved-result-probe.log` | `9145acf4b17c6be740251dbceb602cbeb49f778496254de3e4d771c2e00c7cf3` |
| `saved-result-probe.exit` | `101` |
| `saved-result-initial.log` | Initial same-assertion failure; not the warning-clean final source/log pair |
| `env.sh` | Exact environment below |

## Commands actually run

Setup/read-only inspection included `git worktree add -b
 task/086-final-contract-audit /workspace/shared/minicore-host-task086
 587318b69a2e1f72d0f82999d3f6f6badacfe0e4`, `git rev-parse HEAD HEAD^{tree}`,
`sha256sum SPEC.md`, reading AGENTS.md, the engineering skill, the complete SPEC,
route/owner sources and named test bodies. No repository `.agents/skills` files
were available. Some oversized inspection output was truncated, so relevant
sections were reread in smaller chunks; filenames alone were not used to infer
contract behavior.

The dependency/test environment was:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-task086-target
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
```

Rust 1.90.0 was verified. The **new exclusive target directory** was not shared
with another worktree, avoiding prior stale-rlib evidence contamination.

Executed twice from the fixed-source worktree while the private test was present:

```sh
cargo test -p minicore-host-core --test task086_saved_result_probe \
  --locked --offline -- --nocapture
```

Both exit 101 as documented. Between runs, only the private probe was formatted:

```sh
rustfmt --edition 2024 --config skip_children=true \
  crates/host-core/tests/task086_saved_result_probe.rs
```

For replay in this environment, copy the clean evidence source to the above test
path, source `env.sh`, run the command, and remove only that private copy afterward.
A future source fix needs its own explicit baseline-red/fixed-green validation;
this audit cannot certify such a fix or later Runtime changes.

Final document-only checks are recorded below. No full Rust,
Python, lint or rustdoc gate was rerun by this worker. The coordinator's fresh
908-Rust/4-doc/20-Python strict gate at the fixed baseline is separately attributed
in the audit; it is not the result of this task's failing probe. CI status was not
queried here. All source edits in this task are documentation only.

## Verification and handoff

Document validation passes:

- `python /workspace/shared/task086-audit-evidence/check_docs.py`: exactly 52
  unique matrix methods equal the catalog and real route set; all 84 named test
  references (67 unique) resolve to actual test functions; all relative links
  exist; both documents have clean trailing whitespace/newlines; only the two
  owned files differ; private probe is removed; source tree and SPEC hash match.
  This is a reference-integrity check, not execution of those tests.
- `./scripts/check-whitespace.sh`, `cargo fmt --all -- --check`, and
  `git diff --cached --check` pass. Final commands/results are retained in
  `document-checks.log`; no other Rust/Python/lint/rustdoc gate is implied.
- The protected production tree, manifests, Cargo.lock, SPEC, old coverage,
  workflow and shared documents remain byte-identical to the fixed baseline.
  `git status --short` before commit lists only the two owned added documents.

The documentation-only commit containing this record is reported to the parent
with its exact hash; there is no co-author trailer or remote publication. The
parent integrates the two owned files, assigns/validates the saved-result repair,
and combines remaining diagnostics/Runtime/performance/external-review evidence
at an explicitly named later source. Do not change this fixed-source matrix to
claim a later fix without recording its new source and checks.
