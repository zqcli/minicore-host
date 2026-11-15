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

## Ownership and work plan

All three engineering workers were requested with `gpt-6-astra` and `max`
reasoning. This records the requested configuration, not independent proof of
effective runtime settings.

1. Coordinator: SPEC, root manifests/locks/version, integration review, complete
   validation, exact revision pins and publication checkpoints.
2. `host_config_reference_impl`: owner/core configuration, public provenance,
   persisted sum types and deterministic fingerprints; no storage path ownership.
3. `host_flat_storage_impl`: physical storage, discovery, collision/deletion and
   lease behavior, path defaults/scaffolding, diagnostics and affected fixtures.
4. `tui_config_reference_impl`: TUI integration, initialization, docs/help,
   regression tests and package version; final Host pin is coordinated.

Workers begin with read-only planning and review; implementation starts only
after the specification checkpoint is committed. Shared type/API decisions are
coordinated before editing. Runtime ownership and the existing UI are retained.

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

## Progress and evidence

- Fresh clean checkouts match both recorded remote `dev` heads.
- SPEC rules and current configuration/storage/TUI contracts were inspected.
- SPEC 4.11.1 was published first at `d95ab422`; the full schema-3
  storage and file-reference implementation is not yet present on remote dev.
- The prior 0.12.1 Linux quality-gate failure was traced to
  `repeated_signals_do_not_force_paused_backlog_but_explicit_rpc_force_finishes`.
  The process emitted `host.shutdown_requested` before the owner accepted
  shutdown, allowing a subsequent `host.info` read to see `running`.
  Commit `2c5de6bb` emits the event after successful acceptance; its Linux
  quality gate passed in run 43669197785.
- Commit `f064db49` updates shared startup defaults to independent
  `~/.minicore/config` and `~/.minicore/data` paths, and places scaffold
  logs in a sibling `logs/` directory. Commit `74b4808f` formats the
  updated application path test. Cross-platform CI is pending.
- TUI commits `32399085` and `11973145` flatten command templates and
  make missing `host.json` the first-run condition; the latter fixes the
  formatting failure from the first CI attempt.
- Both repositories still report package version 0.12.1. Do not publish
  0.12.2 or pin the new Host in TUI until schema-3 persistence, references,
  validation and cross-platform CI are complete.
