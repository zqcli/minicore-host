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

- Repository instructions and current SPEC rules read. Clean local dev heads
  match the remote baselines. Three implementation workers are assigned; they
  remain read-only until this SPEC-first commit is created.
- Shared API and ownership boundaries agreed. Implementation and validation
  remain pending at this documentation checkpoint.

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
