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
  home/env, single candidate and agent-defaults template paths.
- Dynamic Agent creation copies only a selected Agent configuration, never its
  declaration metadata, Session overrides or history. No implicit default model.

## Baselines and versions

- Host dev d07286d2cb94e15d670cb81137c50972a6f9f3c3, 0.11.1 / SPEC 4.10.3.
- TUI dev bd6f9ed3d108b9594a7f75d3f2c491bfed898520, 0.11.0, pinned Host 32f074c.
- New public contract: SPEC 4.11.0; Host and TUI target 0.12.0.
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

## Progress

- SPEC 4.11.0 was committed before implementation: local `11ff651c786b199aa3921eec337e95d2bbc035f8`,
  published `be14569cc40bf683468106ca7880102fdaa4999f`, identical tree
  `36442b3b67f514dba8fc6718041b1f122d1e5e0d`. Three requested Astra/max workers
  then began implementation in separate scopes.
- Shared loader, instruction fields, declared Agent ownership and directory
  reload production paths are implemented; integration tests and independent
  review remain in progress.
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
- TUI source/docs are implemented and isolated local-path all-targets compilation
  passed. Scoped tests and final immutable Host pins remain pending.
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
  The native-record preservation regression passed 1/1. Full workspace gate is
  now running; TUI lib tests passed 463 with 3 explicitly ignored benchmarks.

## Verification plan

Target loader, real owner and frontend tests for the accepted boundaries, then
formatting, strict lint/rustdoc, full Host/TUI suites and native CI packages.
Record exact source hashes, commands, outcomes and unverified limits. No live
provider call is required or implied by controlled-model integration tests.

## Issues and resolutions

- Available build disk is low. Remove only reproducible targets from completed
  earlier worktrees; keep active repositories, sources and task evidence.

## Results

Pending implementation, review, validation and publication.
