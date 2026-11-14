# Task 111 — JSON configuration directories and layered instructions

## Goal and authorization

Implement the owner-approved design in minicore-host and minicore-tui, updating
SPEC before code. The user requested parallel gpt-6-astra workers with max
reasoning. This records the requested settings, not independently verified
runtime model settings. Work on dev; preserve history and use zqcli commits
without co-author trailers.

## Accepted requirements

- Standard UTF-8 JSON throughout: host.json, agents/*.json, roles/*.json and
  TUI-owned tui.json. Web configuration belongs to its independent deployment;
  no Web client changes or multi-Host UI implementation are included here.
- Shared crates/host-config reads business files and resolves Role references.
  Host owns actual configuration application, identity, revisions and receipts.
- Stable declared Agent key, separate display name, and role filename reference.
  Bind a declared key to the actual AgentId in owner metadata. Preserve existing
  IDs, Sessions and history, including when releasing file management.
- Directory values win at startup. Explicit reload reads the bound directory,
  without candidate/path supplied by the client. No automatic file watcher,
  migration, fallback or cross-object rollback. Report actual partial results.
- Host, Role and Agent instructions may be empty; compose nonempty system inputs
  in that order. Copy Agent/Role text into Session configuration and freeze each
  Turn. Retain native Session update semantics.
- TUI passes the resolved --config-dir value to the embedded Host, reads its own
  tui.json directly and keeps input history under data-root/tui. Remove old TOML,
  legacy home flags, MINICORE_HOME, single-candidate reload and agent-defaults
  template paths.
- Dynamic Agent creation copies only a selected Agent configuration, never its
  declaration metadata, Session overrides or history. No implicit default model.

## Baselines and versions

- Host dev d07286d2cb94e15d670cb81137c50972a6f9f3c3, 0.11.1 / SPEC 4.10.3.
- TUI dev bd6f9ed3d108b9594a7f75d3f2c491bfed898520, 0.11.0, pinned Host 32f074c.
- New public contract: SPEC 4.11.0; Host and TUI are 0.12.0.
- Remote dev heads were checked against these baselines before editing.

## Ownership and work plan

1. Coordinator: SPEC, task record, Host app CLI, workspace manifests/lock/version,
   integration review, validation, Git publication and CI.
2. config_module_impl: new configuration crate, pure value relocation, protocol
   instruction fields, resource prompt composition, tool configuration changes.
3. host_config_owners_impl: Host/Agent/storage owners, directory startup/reload,
   source binding, transport contracts and owner integration coverage.
4. tui_config_impl: TUI JSON configuration, directory startup/reload, dynamic
   Agent copying, source/tests/docs. Final Host Git pins are coordinator-owned.

## Published checkpoints

| Repository | Change | Published commit |
| --- | --- | --- |
| Host | SPEC 4.11.0 before implementation | [be14569c](https://github.com/zqcli/minicore-host/commit/be14569cc40bf683468106ca7880102fdaa4999f) |
| Host | Directory configuration and owner implementation | [1d2e12d7](https://github.com/zqcli/minicore-host/commit/1d2e12d7cec89e95d47250e3fec9be927c00bb43) |
| Host | Cleanup fixture phase and unwind release | [bf5e59cc](https://github.com/zqcli/minicore-host/commit/bf5e59cc7cea2c4dfce91e4e08a1f40134371e81) |
| Host | Metadata and history-writer fixture barriers | [2f5344d2](https://github.com/zqcli/minicore-host/commit/2f5344d2e8cfb6e49d11cf1b188381264f8ee7fa) |
| TUI | Shared directory and local JSON UI implementation | [2028294f](https://github.com/zqcli/minicore-tui/commit/2028294f8b7a3c227229e33c7149a3f1636b4cb8) |
| TUI | Intermediate Host bf5e59cc pin | [3a641d1f](https://github.com/zqcli/minicore-tui/commit/3a641d1f1c163390186b50bfc21a38614e011f15) |
| TUI | Final validated Host 2f5344d2 pin | [8a6da1ff](https://github.com/zqcli/minicore-tui/commit/8a6da1ffbca9cee6f4a2770fd222726ddd1bce3e) |

The final Host code/test tree is `f19f925495066e6181233d13d9f10fdd75fb5659`.
The SPEC SHA-256 is
`0fe877e0d2809fd6a11b44ff8f90c6aa9029446f3757ea26d393a2ebed9ef016`,
also recorded in AGENTS.md. Publication preserved existing dev history without
force updates; local preparation commits and published commits have identical
trees. Final verification and artifact sources are recorded below.

## Progress

- SPEC 4.11.0 was committed before implementation: local `11ff651c786b199aa3921eec337e95d2bbc035f8`,
  published `be14569cc40bf683468106ca7880102fdaa4999f`, identical tree
  `36442b3b67f514dba8fc6718041b1f122d1e5e0d`. Three requested Astra/max workers
  then began implementation in separate scopes.
- Shared loader, instruction fields, declared Agent ownership and directory
  reload production paths are implemented. Independent source review is complete;
  exact-source native CI and package verification passed for both repositories.
- Host executable uses shared path selection and directory startup. All real
  subprocess fixtures now provide explicit configuration directories. Current
  `cargo check -p minicore-host --all-targets --locked --offline` passed.
- Host workspace packages are 0.12.0 with the new host-config package registered;
  the external package version/source/checksum set is unchanged.
- Example files and package contents now include one complete configuration
  directory. Its actual binary startup created the declared Agent/default Session
  and completed graceful signal shutdown without making a model request. The
  packaging document-link check passed for 22 documents/examples plus binary.
- Host app all-targets tests passed (28 unique cases). The 28 Python script tests
  passed. The config/instructions/resources/tools worker reports 37 scoped cases
  passed; these are not a substitute for the final full workspace gate.
- TUI source/docs are implemented. Its real repository, with immutable Git pins,
  passed formatting, strict Clippy, all-target tests, strict rustdoc and the
  single-crossterm dependency check. Temporary local-path compilation was an
  initial integration aid only and is not in the delivered manifest or lock.
- An independent requested Astra/max review found a shutdown/reload race: the
  accepted directory worker could be rejected by later Host/Agent shutdown
  admission fences. The owner worker corrected that narrow reservation path;
  the independent reviewer rechecked it and the controlled graceful/force
  shutdown regression passed.
- Coordinator review found that omitting serialized empty native instructions
  would discard fields from existing records under the strict storage decoder.
  All four native instruction fields now retain explicit empty strings while
  accepting omitted input. The real store/journal regression confirms supported
  records and history read without changing their bytes; strict decoding is
  unchanged. Independent focused review found no remaining blocker.
- The complete directory regression suite passed 6/6, including actual partial
  save results, independent child creation operation IDs and shutdown ownership.
  The native-record preservation regression passed 1/1. The full local Host gate
  passed at published `bf5e59cc7cea2c4dfce91e4e08a1f40134371e81`, tree
  `58a2e05b9bb69052fceb00336d6bd32acea6a194`: 1,216 unique Rust cases across
  148 test binaries, 7 doctests, 28 Python checks, formatting, strict Clippy and
  strict rustdoc. Independent `--list` output excludes nested probe executions.
- TUI CI source `3a641d1f1c163390186b50bfc21a38614e011f15`, pinned to Host
  `bf5e59cc7cea2c4dfce91e4e08a1f40134371e81`, passed all eight jobs in
  [run 43587637443](https://github.com/zqcli/minicore-tui/actions/runs/43587637443):
  Linux quality, Linux/macOS/Windows native tests and four package targets.
  Its four downloaded artifact digests and actual BUILD.txt source pins were
  verified. Final TUI source `8a6da1ff` changes only the immutable Host pin in
  the manifest, lock and changelog; all 13 Host packages use 0.12.0 at `2f5344d2`,
  with no other locked package changes. Exact-source CI passed all eight jobs.

## Verification approach

Targeted loader, real owner and frontend tests cover the accepted boundaries,
followed by formatting, strict lint/rustdoc, full Host/TUI suites and native CI
packages. Exact source hashes, commands, outcomes and unverified limits are
recorded below. No live provider call is required or implied by controlled-model
integration tests.

## Issues and resolutions

- The first local full gate exhausted build disk while linking. Rebuild using
  CI's existing `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0` and
  `CARGO_PROFILE_TEST_DEBUG=0` settings passed; no repository profile or
  dependency change was used to hide the infrastructure failure.
- An old cleanup test asserted a terminal `Applied` result while a real directory
  reload was still `Applying`. Its assertion panic also left the controlled
  resource-drop gate blocked. Test-only commit `bf5e59c` checks the actual
  intermediate phase and releases that gate during unwinding. The focused case
  and subsequent full local gate passed. The original
  [CI run 43585486678](https://github.com/zqcli/minicore-host/actions/runs/43585486678)
  logged the same long-running test in all five jobs before they were cancelled;
  remote logs did not expose the original panic, which was reproduced locally.
- CI run 43587363372 exposed a separate existing App fixture issuing
  `session.update` before input-activity metadata saving completed. It now
  observes `session.state.metadata_saving=false` before its original single
  update. The target and all four `ordinary_tools` cases passed; there is no
  automatic mutation retry or production change.
- The same CI run exposed Windows correctly denying raw history-file reads while
  the Host writer retained its lock. The directory restart fixture now reads
  bytes only after its existing shutdown and explicit Host/connection drops,
  retaining all identity, Session-instruction and byte-preservation assertions.
  Both fixture corrections received independent source review and are published
  together at `2f5344d2`. The affected suites passed locally: `ordinary_tools`
  4/4 and `configuration_directory` 6/6. Their two target cases also passed
  separately. Final native CI validates this exact published source.

## Validation commands and limits

Host's full gate is `./scripts/check.sh`, with `CARGO_BUILD_JOBS=2`,
`CARGO_NET_OFFLINE=true` and the three CI profile variables above. Unique cases
are counted using `cargo test --workspace --all-targets --locked -- --list`.
All 225 external locked Host package versions, sources and checksums match the
baseline. TUI uses `cargo fmt --all -- --check`,
`cargo clippy --locked --offline --all-targets -- -D warnings`,
`cargo test --locked --offline --all-targets`, strict `cargo doc --no-deps`,
`cargo tree --locked --offline -p crossterm`, and independent test listing.

The last full local TUI gate ran at `3a641d1f`, pinned to Host `bf5e59cc`, and
passed 472 default cases with four harness-level ignores: three optional
performance cases and one subprocess helper that its passing integration test
executes separately. There are 476 uniquely listed cases; the child helper
output is not counted twice. The final pin has formatting, whitespace and lock
comparison checks, and its exact-source CI supplies the complete final gates.
No live provider call is claimed. Provider integration uses actual
Host/resources/transports with explicit controlled loopback HTTP/SSE model
responses. Web is outside this task.

## Results

Production implementation and independent review are complete. The final Host
source is `2f5344d2e8cfb6e49d11cf1b188381264f8ee7fa`; the final TUI source is
`8a6da1ffbca9cee6f4a2770fd222726ddd1bce3e`, tree
`c4814837b507d35fbc2f18e3d2426a09058c75de`, pinned to that same Host revision.

### Host native validation

[Run 43588511364](https://github.com/zqcli/minicore-host/actions/runs/43588511364)
passed the Linux full quality gate and all four native platform gates at the
exact source above. Each platform independently lists its unique cases; nested
probe output and repeated standard-user executions are excluded.

| Native platform | Unique Rust cases | Doctests | Job |
| --- | ---: | ---: | --- |
| Linux x64 | 1,216 | 7 | [132428050751](https://github.com/zqcli/minicore-host/actions/runs/43588511364/job/132428050751) |
| macOS ARM64 | 1,208 | 7 | [132428050972](https://github.com/zqcli/minicore-host/actions/runs/43588511364/job/132428050972) |
| macOS x64 | 1,208 | 7 | [132428051047](https://github.com/zqcli/minicore-host/actions/runs/43588511364/job/132428051047) |
| Windows x64 | 1,193 | 7 | [132428050968](https://github.com/zqcli/minicore-host/actions/runs/43588511364/job/132428050968) |

The [quality job](https://github.com/zqcli/minicore-host/actions/runs/43588511364/job/132428050913)
passed formatting, strict Clippy/rustdoc, workspace tests, 7 doctests and all
28 Python checks. All three previously failing synchronization cases explicitly
passed in the final native logs. Windows additionally passed five Storage/Host
cases under an actual non-administrator account with a loaded profile and denied
executable writes; the temporary account was removed after success.

### TUI native validation

[Run 43589491558](https://github.com/zqcli/minicore-tui/actions/runs/43589491558)
completed successfully on attempt 1 at `8a6da1ff`, with all eight jobs: Linux
format/Clippy/rustdoc/dependency quality, native Clippy/tests on Linux, macOS and
Windows, and four release-package targets.

| Native platform | Unique cases | Default passed | Harness ignores | Job |
| --- | ---: | ---: | ---: | --- |
| Linux | 476 | 472 | 4 | [132431001406](https://github.com/zqcli/minicore-tui/actions/runs/43589491558/job/132431001406) |
| macOS | 476 | 472 | 4 | [132431001380](https://github.com/zqcli/minicore-tui/actions/runs/43589491558/job/132431001380) |
| Windows | 475 | 471 | 4 | [132431001383](https://github.com/zqcli/minicore-tui/actions/runs/43589491558/job/132431001383) |

Counts come from the actual test logs and correspond to 1,415 passing default
executions, or 477 cases after cross-platform target/name deduplication. Windows
has one platform-specific case in place of two Unix-only cases. The subprocess
helper is executed by passing integration cases; only the three opt-in
performance tests were not run. Final pin checks did not change external
dependencies; the initial JSON migration removed seven obsolete TOML packages
without adding or upgrading other external packages. Crossterm remains 0.29.0.

### Packages and completion

Host run 43588511364 completed successfully on attempt 1 with all nine jobs.
Each native Host package passed its actual executable smoke check and archive
readback with exactly 23 files, including the complete configuration example,
the configuration guide and one Host executable matching the tested binary's
SHA-256. The package API associates every artifact with `2f5344d2`.

All four TUI artifacts were downloaded and their ZIP SHA-256 values matched the
GitHub artifact digests. Their actual BUILD.txt files consistently record
TUI `8a6da1ffbca9cee6f4a2770fd222726ddd1bce3e` and embedded Host
`2f5344d2e8cfb6e49d11cf1b188381264f8ee7fa`. The macOS x64 TUI binary is built
on the macOS ARM runner; the TUI's macOS native test job runs on that ARM runner.

| Package target | Host 0.12.0 artifact | TUI 0.12.0 artifact |
| --- | --- | --- |
| Linux x64 | [14111818485](https://github.com/zqcli/minicore-host/actions/runs/43588511364/artifacts/14111818485) | [14111788357](https://github.com/zqcli/minicore-tui/actions/runs/43589491558/artifacts/14111788357) |
| macOS ARM64 | [14111898132](https://github.com/zqcli/minicore-host/actions/runs/43588511364/artifacts/14111898132) | [14112165521](https://github.com/zqcli/minicore-tui/actions/runs/43589491558/artifacts/14112165521) |
| macOS x64 | [14112161036](https://github.com/zqcli/minicore-host/actions/runs/43588511364/artifacts/14112161036) | [14110953777](https://github.com/zqcli/minicore-tui/actions/runs/43589491558/artifacts/14110953777) |
| Windows x64 | [14111374472](https://github.com/zqcli/minicore-host/actions/runs/43588511364/artifacts/14111374472) | [14112280052](https://github.com/zqcli/minicore-tui/actions/runs/43589491558/artifacts/14112280052) |

A final bounded documentation review found the Host/TUI guides and examples
consistent with implemented paths, first-run behavior, identities, instruction
layers and reload rules. Host's repository example intentionally selects no
tools/workspace and uses stderr; the TUI initializer instead generates explicit
tools/workspace and a file log suitable for an interactive terminal. Both need
actual provider configuration and the named credential environment variable.

The completion commit changes only PROJECT.md and this task record and uses
`[skip ci]`; verified implementation, tests and package sources are unchanged.
Development remains on dev. No release tag, GitHub Release or main merge was
requested or performed. No implementation or required validation remains open.
