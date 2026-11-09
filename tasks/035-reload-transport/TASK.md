# Task 035: Connect canonical reload to Host transports

## Scope and fixed inputs

- Worktree `/workspace/shared/minicore-host-reload-transport`, branch `feat/canonical-reload-transport`, fixed integration base `cdb8ee144ef92ff29c1025148e05b5a9bf50f2e3`.
- Only `crates/host-transport/**` and this task report are owned here. Core, protocol, resource assembler, manifests, lockfile, PROJECT.md and TUI remain unchanged. No push.
- Follow unchanged SPEC v4.93 (`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`), particularly configuration atomicity, operation rights, direct-owner receipts and accepted work surviving timeout/disconnection.
- Canonical core API was independently reviewed at `a80311c10c1d6cb8b0634034e929aadca4d93994` and is already integrated in this base. Initialization-only preparer, full candidate DTO, direct owner receipt lookup and configuration publication remain owned by that implementation.
- Runtime remains the real fixed `69faf9196991524b5549499d6208a1eaf567bf56` (0.10.0); no patch/path override or Runtime change. TUI remains untouched at its existing reference `3beeb1d8a525e58ce6352e05061f242c5580e248`.

## Implementation

`HostClient::reload(HostReloadRequest<HostConfig>, CallOptions)` reuses the existing typed operation table, canonical `Host::reload`, one client-side monotonic deadline, and common JSON-RPC dispatcher for duplex stdio and real TLS WebSocket. The full result is the actual HostReloadReceipt. Business Faults remain errors, accepted preparation/save failures remain receipt facts, and existing `host.receipt.get` reads current direct-owner facts. No new protocol type, transport state, cache, retry, recovery endpoint or dynamic preparer setter is added.

The route is included among 35 supported adapters. Empty reload parameters are now invalid, rather than being classified as an unimplemented route. Low-level Hosts without a preparer still return HostMethodUnavailable for valid reload requests. Existing low-level prepared publication APIs remain supported and their existing tests are retained.

## Code reference, not compatibility baseline

Consulted `minicore-agent` at `f36a87366d8e2fe0a1c8216463887902e471c739`: `src/rpc/server/dispatch.rs`, `src/rpc/server/tests/discovery.rs`, `src/rpc/server/tests/review.rs`, `src/agent.rs` and its reload tests. Useful reference patterns are strict params, reservation before asynchronous preparation, immutable active execution bindings and genuine RPC harnesses. The current Host transport already implements the shared decoder/dispatch/deadline infrastructure, so this stage reuses it directly instead of importing a second stack.

The older `agent.reload` takes empty params, reloads a retained startup file, defers application in the RPC layer, returns `{ok:true}`, and updates existing Session future configuration. Those behaviors are not copied: Host v4.93 requires an explicit candidate and operation/revision fields, Host-owned accepted work and receipts, network-key invalidation, and unchanged existing Session choices. Agent code is reference only, not the implementation baseline or a compatibility requirement.

## Validation in progress

- Initial targeted runs pass six canonical contract cases across typed/duplex/WSS paths, three real-transport timeout/disconnection cases, and two canonical key-publication/production HTTP-SSE cases.
- The first focused run exposed a test-only teardown hang: dropping one Tokio duplex split writer did not signal EOF while its reader remained alive. The raw test peer now explicitly shuts down its writer before joining the server. That initial run was interrupted; a rerun passes. No production behavior was changed to conceal the test defect.
- Full transport suite, workspace aggregate gates, exact test inventory, repeated execution and final commit checkpoints will be recorded after completion.

Environment for all Rust checks:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-reload-transport-target
```
