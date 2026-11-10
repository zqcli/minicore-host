# Project organization and 0.9.20

## Scope and authorization

The owner approved the organization review and explicitly requested parallel
implementation with Astra/max workers, final review, version **0.9.20**, and
publication to `dev`. The fixed starting source is
`c4451653ea5f3110041bef85a5b86a55b1b203a4` (0.9.10), verified against the connected
GitHub `dev` ref before implementation. Existing remote history must be retained.

SPEC v4.93 remains byte-for-byte unchanged, SHA-256
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
The production Runtime pin remains 0.10.0 at
`3aa47629929906832dcb309afb7d242c1ae3e53f`.

## Implementation groups

- Core: input/execution, loading/close, query/observation, Turn bindings,
  compaction prompt adapters, typed shutdown phase, and core test organization.
- Transport/protocol: typed clients, RPC framing/server/client, shared method
  identifiers, functional protocol families, and semantic query test modules.
- Tools/resources: local binding, inspect projections, production resource
  preparation, and shared production fixtures.
- Integration: workspace inheritance, storage validation/read organization,
  workspace edit evidence, cross-crate test support, documentation, independent
  review, release validation, and publication.

Three disjoint implementation worktrees were started with requested model
`gpt-6-astra` and requested reasoning effort `max`. This records the requested
configuration, not independently attested effective runtime settings.

## Deliberate boundaries

Keep the 12-crate layout for this release: the existing compaction package is
still a useful compile-time boundary and its removal would change public API.
Retain trusted embedding network hooks and thin HostSettings conveniences;
update their guidance to the actual configured/reload implementation. Preserve
historical performance scripts byte-for-byte because retained measurements
verify their hashes. Keep transport global-subscriber diagnostic tests in
separate processes. Do not add lifecycle managers, caches, actors, quotas,
cross-owner shortcuts, or new persistence/recovery behavior.

## Implementation and review

The release source checkpoint is
`fa2c40043695f493cfbfbf057ec8145ec8f5f9c7`. Subsequent task/project records change
documentation only; Rust source, tests, scripts, manifests, lockfile and SPEC
remain identical to this validated checkpoint.

| Scope | Implemented organization |
| --- | --- |
| Workspace | Package metadata and dependency declarations inherit from the root; member features and the storage protocol dependency's exact requirement retain their semantics |
| Core | Session owner/state remains in `session.rs`; input, execution, history, loading, close and query are explicit modules on the same owner |
| Capabilities | Active Turn bindings are separate from already accepted delegation delivery/cleanup permits |
| Compaction | Prompt/history adapters are separate; manual and automatic work retain one commit path |
| Shutdown | Existing strategy type replaces the numeric phase, preserving graceful-to-force progression and accepted-create handoff |
| Transport | Typed connection/Host/Session/subscription clients and RPC framing/wire/server/client are grouped by responsibility; pending/setup/Drop correlation remains together |
| Protocol | Existing DTO families are colocated, ToolList DTOs live in protocol with transport reexports, and operation entries reuse the existing Method enum |
| Tools/resources | Local binding and recording are colocated; private inspect projections select explicit fields; builder/resolver/prompt share precise declaration lookup without executable compaction tools |
| Storage/workspace | One validator is shared by accepted writes and decoded history; saved reads and Git observations are separate from retained tool-edit evidence |
| Tests | Shared provenance assertions moved once to workspace support; production fixtures use semantic directories; five owner-diagnostic roots became one suite with all 22 cases |
| Documentation | README points to PROJECT for current status; historical audit source links use the matching published tree, and existing crate guides describe the new organization |

Independent cross-review is recorded in [review.json](review.json). The core
review compared execution/close/loading bodies, state definitions, all 344 test
bodies and attributes, and explicitly reviewed terminal-save/manual-prompt
extractions. Transport review verified 52 routes, 51 ordinary handlers, strict
DTO tokens and pending/subscription lifetimes. Tools review checked exact JSON
omission/null and current-versus-saved result shapes. Workspace/storage review
checked the dependency graph and unchanged write/read ordering. All scopes were
approved without blocking findings.

## Validation

The complete release gate passed:

```sh
CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 ./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
PYTHONDONTWRITEBYTECODE=1 python3 tasks/090-fresh-performance/verify_evidence.py
PYTHONDONTWRITEBYTECODE=1 python3 tasks/098-composed-performance/verify_evidence.py
```

The pinned Cargo toolchain was added to PATH for these commands. Results:

- 956 unique Rust cases, counted from the independent `--list` output; nested
  storage subprocess probes are not additional cases.
- 4 doctests and 28 Python checks; formatting, Clippy with warnings denied,
  strict rustdoc and whitespace checks pass.
- 126 Cargo test targets, 123 nonempty; workspace metadata contains 114
  integration binaries, 11 libraries and one executable.
- All 955 prior cases remain; the one new real-owner regression covers Message
  text/reply projection and full/summary/unfinished ToolCall output shapes.
- Historical Task090/Task098 raw evidence, script hashes and regenerated
  summaries still match. These are retained measurements, not new benchmarks.
- Before the version bump, all 221 packages' dependency metadata and the full
  graph matched the baseline. The bump changes only the 12 local lock versions
  and 34 internal version requirements; external sources/features are unchanged.

The first full gate attempt stopped during test linking when the workspace ran
out of disk space. No complete Rust-test pass was claimed from it. After the
compilers exited, reproducible incremental caches were cleared and the same
source passed the entire gate. The failed attempt and passing rerun have
separate log hashes in [validation.json](validation.json).

Final documentation changes receive separate whitespace and local-link checks.
No live external-provider, consuming TUI or new performance acceptance is claimed.

## Accounting

| Physical Rust lines | Baseline | Release source | Net |
| --- | ---: | ---: | ---: |
| Non-test-source files | 40,663 | 41,111 | +448 |
| Explicit tests/support | 71,592 | 71,879 | +287 |
| Total | 112,255 | 112,990 | +735 |

Counts include comments and blank lines. Explicit test/support classification
uses test paths and test-only filenames; non-test-source files can still contain
inline tests. Rust files increase from 330 to 364 because mixed responsibilities
are separated, while integration test binaries decrease from 118 to 114.
The Session root decreases from 1,823 to 438 lines. These figures describe
organization, explicit projections and test coverage, not a measured speedup.

## Publication

The owner authorized publication to `dev` after review and validation. The
connected GitHub Git Data API reproduces all nine source checkpoints in order,
with identical trees and original commit messages. Actual author and committer
are both `zqcli <276883664+zqcli@users.noreply.github.com>`; no co-author trailers
are present. API-generated timestamps produce different commit IDs, recorded
with source/tree/parent evidence in [publication.json](publication.json).

- Validated release code `fa2c40043695f493cfbfbf057ec8145ec8f5f9c7` corresponds to
  [`7152b1be236497630d79e96d64dc57316c8040db`](https://github.com/zqcli/minicore-host/commit/7152b1be236497630d79e96d64dc57316c8040db).
- Source plus review/validation records `9dd7df53361c9fe0c282610f8bf0036e273183d3`
  corresponds to
  [`9b2e851a810ccf19086ef4f7001b1b24dd170ed8`](https://github.com/zqcli/minicore-host/commit/9b2e851a810ccf19086ef4f7001b1b24dd170ed8).
- The final publication-record commit only adds these links and the mapping;
  production source, tests, scripts, manifests, lockfile and SPEC remain identical
  to the validated release code.

The branch update preserves the original remote head
`c4451653ea5f3110041bef85a5b86a55b1b203a4` as an ancestor and uses that observed
expected SHA with `force: false`. Hosted CI is checked for the resulting `dev`
head; the passing local gate above is a separate result. This release does not
merge `main`, publish crates or claim live-provider/TUI acceptance.
