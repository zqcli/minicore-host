# Task 017: Authenticated WSS transport

Status: WSS authentication/library stage validated; follow-up review fixes and
observation integration remain in progress. Implements the library access/connection slice of SPEC §§2,
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
- `587f239` implements Host-owned authentication and atomic key publication.
- `1d00fe7` implements shared framed RPC plus actual WSS client/server and loopback tests.
- `22bd2da` preserves unknown Fault leaves and registers the observation worker's seven leaves.
- `48d06a0` mechanically migrates registered-kind callers to explicit `Some` checks.
- `ab4a56b` boxes optional detail payloads to keep ordinary Fault results compact.

## Validation

- `cargo check --workspace --all-targets --locked`: passed after initial production
  implementation against pinned Runtime `18a6ac9` (0.9.2).
- `cargo test -p minicore-host-core --lib --locked network_tests`: passed, 5 tests.
- `cargo test -p minicore-host-transport --all-targets --locked`: passed, 32 tests
  (15 existing, 1 quota configuration test, 12 real loopback tests, 4 TLS wait tests).
- `cargo clippy -p minicore-host-transport -p minicore-host-core --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --all-targets --locked`: passed, 312 unique tests.
- `cargo test --workspace --doc --locked`: passed.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`: passed.
- `cargo test --workspace --all-targets --locked -- --list`: 312 unique test cases;
  subprocess probe output is not counted again.
- `cargo fmt --all --check`: passed.

The large-frame regression uses a >16 MiB correlation ID and a real `host.info`
response, proving bidirectional framing independently from Runtime 0.9.2's text
limit. A first attempt using >16 MiB business text received the actual old
Runtime-related `SESSION_INPUT_INVALID` rejection; no WSS quota was involved.
- Runtime 0.10 migration and shared observation wiring remain separately owned.

## Integrated checkpoint

Topic-to-dev mapping: `795b233` -> `3309659`, `37e40c2` -> `502b38a`,
`587f239` -> `c5634f4`, `1d00fe7` -> `bd8bf2f`. Observation-only bridge commits
were not integrated without their actual core observation module.

Fixed dev `bd8bf2f5f9306d8d229f1907435cc22e849583d1` passed
`./scripts/check.sh`: committed/staged/working whitespace, formatting, strict
all-target workspace Clippy, all unit/integration tests, 3 doctests and strict
rustdoc. `cargo test --workspace --all-targets --locked -- --list` counted
**321 unique tests**. The 9 workspace-review regressions account for the
difference from this topic's 312-case source baseline. Runtime remains the
published 0.9.2 revision; no local source override was used. Logs:
`wss-integrated-bd8bf2f.log` and `wss-test-list-bd8bf2f.txt` in the local
validation directory.

After validation, the accumulated main-checkout build cache was cleaned to
recover shared disk space; source, commits and validation logs were retained.
Subsequent fresh builds disable debug/incremental artifacts through environment
settings, not repository behavioral changes.

The earlier typed/stdio review still has two follow-ups: preserving unknown
Fault leaves (including nested receipts) and allowing the client wait budget to
progress during synchronous creation validation. Their separate fixes are not
in this checkpoint. This WSS gate is not independent full-product approval, a
production process/config loader, completed `host.reload`, or TUI/live-model
validation.
## Independent review follow-ups

The existing typed/stdio review found that unrecognized Fault leaves were
rejected as malformed responses, including faults nested in accepted receipts.
The owner approved a first-version API correction: `Fault::kind()` returns
`Option<FaultKind>`, while `type_name()`, `code()`, message and opaque details
retain unknown wire facts. Registered mappings and detail schemas stay strict.
The API change and required mechanical callers are separate adjacent commits;
whole-workspace checks require both. Three new protocol regressions pass.

Unknown-fault regressions now cover protocol round trips, strict known mapping
rejection, stdio top-level errors with unrelated concurrent queries, accepted
message/start-error receipts, real partial Agent creation, and WSS error/result
frames. Focused protocol/transport tests and whole-workspace clippy with warnings
denied passed after the compatibility change. No transport-specific business
error fallback or duplicate fault registry was introduced.
