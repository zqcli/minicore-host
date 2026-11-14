# Task 110 — Host workspace directory picker

## Authorization and baseline

- Owner authorized separate minicore-web with its existing UI and native Host WSS;
  add `workspace.roots`, `workspace.list`, `workspace.mkdir`, then release Host 0.11.1.
- Host input: dev `32f074c57868533af81b0c6491440bae24b872c8` (0.11.0).
- Design first: SPEC 4.10.3, SHA-256 `a9bec7b69a94082e7ea4d156f82f58361c241cb0d7ed9c01935f80b3bee9b792`.
- Mature directory reference: minicore-tui
  `2451d7289214385e3d21b46174b36f32a41dce45:src/remote/fs.rs`; its fixed
  MAX_ENTRIES quota is deliberately excluded under SPEC Rule 12.
- Requested engineering worker: gpt-6-astra, max reasoning. Effective runtime
  configuration is not independently attested.

## Scope and ownership

Host configuration owns explicit `workspace_roots`; Host owns directory requests,
accepted mkdir and receipts; host-workspace performs actual filesystem work; the
shared typed operation table supplies direct, stdio and WSS calls. No Session is
needed for selection. Existing connection, approval, Runtime and TUI contracts
remain unchanged. No compatibility layer, implicit root, quota, retry or generic
operation manager is introduced. Root agent owns publication and independent review.

## Progress and evidence

- 2026-11-14: read repository AGENTS, SPEC rules and relevant owner, resource,
  workspace and transport paths. Contract coordinated before implementation.
- SPEC-first commit: pending. Implementation and validation follow that commit.
- Planned checks: real roots/list/mkdir, empty roots, authorization and operator
  rights, scoped paths/symlinks, pagination/errors, platform path handling, actual
  write lifetime, receipt readback, shared transport operations, fmt/Clippy/tests/docs.
- Publication is not performed by this worker.
