# Task 017: Authenticated WSS transport

Status: In progress. Implements the library access/connection slice of SPEC §§2,
10–11, not Host process/config assembly or production CLI/TUI completion.

## Scope and ownership

- Actual TLS WebSocket listener/client with supplied rustls configurations.
- One Host ConnectionRegistry owns configured network credentials, authenticated
  connection registration, operation rights, and atomic revocation.
- Shared typed/stdio/WSS business dispatch, result/fault mapping, and one client
  monotonic response deadline. No application frame/message/buffer quotas.
- Trusted configuration/publication hook only; no fake `host.reload` endpoint.
- Fixture-only certificates and model resources; no live provider or credential.
- No remote push, main merge, TUI edits, or Runtime pin change.

## Commits

- `795b233` reserves the Host network module and releases shared `host.rs`.
- `37e40c2` pins TLS/WebSocket/test-certificate dependencies and isolates lock changes.

## Validation

- `cargo check --workspace --all-targets --locked`: passed after initial production
  implementation against pinned Runtime `18a6ac9` (0.9.2).
- `cargo test -p minicore-host-core --lib --locked network_tests`: passed, 5 tests.
- Formatting, full lint/test/doc checks and unique test enumeration are pending.
- Runtime 0.10 migration and shared observation wiring remain separately owned.
