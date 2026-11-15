# Task 115: configuration references and flat persistent storage

## Goal and authorization

On 2026-11-15 the repository owner approved the discussed directory and
reference-only configuration design, explicitly abandoned declaration-file
recovery, and requested SPEC first, then Host and TUI implementation with a
0.0.1 version increment. Both start at 0.12.1 and target 0.12.2 on `dev`.
Existing authorization permits publication to `dev`; no `main` merge, release
tag, GitHub Release or independent Web changes are part of this task.

## Starting checkpoints

- Host: `c7bd16b4e36c94f587aa4802b3298617dc088478`.
- TUI: `555ba1330b146cb40e8938611ae6cf8b7788d66c`.
- Previous design: SPEC 4.11.0.
- Target design: SPEC 4.11.1, committed before implementation.
- Object record schema becomes 3; history schema remains 3.

## Accepted contract

- Default paths are `~/.minicore/config`, `~/.minicore/data` and the scaffolded
  `~/.minicore/logs/host.log`; config/data overrides are independent.
- TUI reads its own `tui.json`, passes the resolved directory to Host, and uses
  `config/commands/`. Missing `host.json` creates only missing starter files and
  exits; existing Host configurations do not gain sample Agents automatically.
- File-managed Host and Agent records contain a source and SHA-256 fingerprint,
  not another copy of configuration bodies. Stable declaration keys bind Agent
  identities. Fingerprints cover complete typed Host configuration or Agent
  name plus fully resolved configuration, including Role contents.
- Independent programmatic Host records and dynamic Agent records retain inline
  configuration. Constructor/source mode mismatch is rejected, not converted.
- Missing bound declarations fail startup/reload preparation before publication.
  No automatic unmanage, snapshot recovery, Session-derived defaults or detach
  API. Failed reload keeps the current effective memory state.
- Explicit Agent deletion follows existing lifecycle and exact child scope;
  it does not modify declaration files. Remove the declaration and explicitly
  delete the Agent before reloading to remove it permanently.
- Persistent files are `data/host.json`, `data/agents/<id>.json`, and
  `data/sessions/<id>/{session.json,history.jsonl}`. One metadata scan groups
  Sessions by their recorded Agent; direct owners still initialize and manage
  children. Session ID conflicts never overwrite existing records.
- Session effective configuration and historical data remain independent;
  active Turns keep their prior bindings. Existing partial-save semantics and
  actual failure reporting remain in effect.
- Old object formats and nested layout are explicitly unsupported. No migration,
  automatic rewrite, dual writer or fallback is added; old files are preserved.

## Implementation ownership

The earlier plan requested engineering workers with `gpt-6-astra` and `max`.
The completion environment exposes no callable subagent/model-selection tool;
implementation, review and validation here were performed directly. Do not
interpret the earlier requested settings as verified effective execution.

Host owners apply resolved configuration, storage owns physical record access,
and TUI owns local preferences. The existing Runtime and terminal UI are retained.
No independent Web, migration, detach or recovery interface was added.

## Validation plan

- Verify native file records omit managed config bodies; dynamic records remain
  inline, identical restart preserves IDs/revisions, Role changes update default
  fingerprints without rewriting existing Session settings.
- Verify missing declarations fail before writes/publication, running reload
  preserves applied state, explicit deletion has exact scope, and retained
  declarations recreate entities through normal owner creation.
- Verify flat discovery, cross-Agent Session collisions, corrupted ownership,
  default Session references, history/journal leases and exact deletion scopes.
- Verify separated path defaults, independent overrides, first-run creation with
  pre-existing empty directories, no overwrite and no unexpected sample Agents.
- Run repository formatting, strict Clippy, native tests, doctests, rustdoc,
  Python/documentation checks and unique test enumeration with locked deps.
- Run existing Linux/macOS/Windows CI and package checks on published revisions.
  Controlled provider tests are not described as live upstream validation.

## Implementation and verification

- SPEC 4.11.1 was published first at `d95ab422`. Completion starts from Host
  `67e19d31d61ee3038aeecf92daed02c4d602e2f4` and TUI
  `1197314521430b7e72e36e25de6e4a45cdc5945e`, preserving prior fixes.
- Both package versions are 0.12.2. TUI pins the exact published Host revision;
  local path patches are validation-only and are not shipped.
- Stored schema-3 sum types distinguish file references from inline values.
  The generic Host decoder preserves exact arbitrary-precision JSON instead of
  routing numbers through serde's intermediate tagged-enum content buffer.
- Startup validates bound declarations and global Session ownership before
  writing configuration metadata. Same-value restarts preserve revisions;
  changed Role defaults affect new Sessions without rewriting existing ones.
- A single metadata discovery pass groups Sessions by their recorded Agent.
  Identifiable per-Session configuration/history faults remain local; unreadable
  ownership, orphaned records, conflicting IDs and legacy layouts fail discovery.
- Session creation exclusively reserves its ID directory. Metadata saves update
  existing records only; collisions and partial creations are not overwritten or
  retried. Explicit deletion retains its exact admitted child scope and leases.
- Native-home defaults and log scaffolding are independent of config/data
  overrides. TUI reads only its own UI JSON, with flat command templates and
  no Host-owned UI-file interface. Diagnostics protect both flat object subtrees.
- The prior signal test race was fixed at `2c5de6bb`: the process emits the
  shutdown-request diagnostic only after the Host accepts the transition. The
  original lifecycle assertions remain intact.

### Local checks

Rust 1.90.0, locked dependencies, Linux x86_64, an ordinary user and an isolated
home directory were used. The container's PID 1 does not reap orphaned shell
children, so an external test launcher acted as a child subreaper; production
code and process-cleanup assertions were not changed for that environment.

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
```

Host: full gate passed; 1,240 native test cases enumerated, seven doctests and
28 Python checks. Nested storage subprocess-probe output is not counted twice.
Formatting, strict Clippy and strict rustdoc passed.

TUI used a temporary local Cargo patch to the exact prepared Host tree for:

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
cargo test --locked --all-targets -- --list
```

TUI: 487 cases enumerated; default run passed 483 with four ignored entries
(three optional performance checks and the subprocess helper invoked by the
integration cases). Native embedded Host tests cover chat/responses, directory
reopen, source identity, flat files, reference-only records, reload failure and
accepted-work shutdown. Snapshot changes are limited to the visible version.

### Publication boundary

The normal CI workflows are retained unchanged. Their exact delivered
checkpoints were independently verified on 2026-11-15:

| Repository | Delivered 0.12.2 checkpoint | CI result |
| --- | --- | --- |
| Host | `2f149145304c7cf6fd3271da937000b6e8593cbd` | [Run 43676489905](https://github.com/zqcli/minicore-host/actions/runs/43676489905): all 9 jobs passed, attempt 1 |
| TUI | `a7b2c0fab2fe43aa56a404a52364a4e351c2096b` | [Run 43676748923](https://github.com/zqcli/minicore-tui/actions/runs/43676748923): all 8 jobs passed, attempt 1 |

Host checks include Linux quality, native Linux/macOS ARM/macOS Intel/Windows
tests, standard-user Windows storage checks and four platform packages. TUI
checks include quality, Linux/macOS/Windows tests and four platform packages.
All 13 native Host entries in the delivered TUI lockfile pin the exact Host
checkpoint above at version 0.12.2. Both workflow conclusions and each job were
read directly from GitHub Actions, rather than inferred from local tests.

No external live model-provider test or interactive terminal manual acceptance
is claimed for this task; integration uses real owners and controlled loopback
HTTP/SSE at the provider boundary.

Temporary source/dependency export tooling is removed from the delivered tree.
Public source files and pinned dependency archives were used only for isolated
validation; no credentials or private runtime data were copied into artifacts.

### Revalidation after subsequent deliveries

The original parallel candidate remained unpublished when its publication call
was interrupted. On resumption, `dev` already contained this completed delivery
and later work: Host `de8c5ad2b3d3d7b6605d789d5fa3f9f5ae33d599` (0.12.5,
SPEC 4.11.4) and TUI `27d6073e414098704b692b9f7040e529a08426ec` (0.12.6).
The older candidate is not applied over those commits and versions are not
repeated or rolled back.

Three follow-up read-only reviewers were requested with `gpt-6-astra` and `max`
for configuration owners, storage/path behavior and TUI integration. They
confirmed the approved Task115 behavior in the current sources, existing test
coverage and configuration guides; they did not rerun or claim new native
tests. Optional performance observations and external native-record corruption
scenarios were not turned into an unrelated rewrite of this completed task.

Current TUI pins Host implementation `355d4b8bac4a3a634cd83a49fbf33fea7b67a262`;
the later Host head differs only in Task118 documentation. The current source
checkpoints also have successful complete CI: Host
[43726944284](https://github.com/zqcli/minicore-host/actions/runs/43726944284)
(9 jobs) and TUI
[43733717562](https://github.com/zqcli/minicore-tui/actions/runs/43733717562)
(8 jobs), both on attempt 1. This closeout updates only PROJECT.md and this task
record, with no source, schema, version or dependency change.
