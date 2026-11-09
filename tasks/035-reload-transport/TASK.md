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

## Validation and final source

- Production route/client checkpoint: `e30aaefe1406978c2a7cff660f58290ed5a65a47` (`feat(transport): route canonical Host reload requests`). Test/functional source checkpoint: `fc8975e12155ccd950cce197bb40c7096592e5f7` (`test(transport): verify reload receipts and accepted work across connections`). This report-only completion follows those two commits.
- At exactly fc8975e, `./scripts/check.sh` passes in full: whitespace, formatting, strict all-target workspace Clippy, every unit/integration target, **three doctests**, and strict rustdoc. Log: `/workspace/shared/minicore-host-reload-transport-artifacts/task035-full-gate.log`.
- `cargo test --workspace --all-targets --locked -- --list` reports **576 listed / 576 unique target-test cases**. Nested storage subprocess output is not counted again. Log: `/workspace/shared/minicore-host-reload-transport-artifacts/task035-inventory.log`.
- `cargo test -p minicore-host-transport --locked` passes the entire transport suite and its doctest. Log: `/workspace/shared/minicore-host-reload-transport-artifacts/task035-transport.log`.
- Eleven new canonical cases pass: six contract cases across typed/duplex/WSS paths, three real-transport timeout/disconnection cases, and two canonical key-publication/production HTTP-SSE cases. `cargo test -p minicore-host-transport --test reload --test reload_wait --test reload_network --locked` passes **ten consecutive repetitions** at fc8975e. Log: `/workspace/shared/minicore-host-reload-transport-artifacts/task035-repeated.log`.
- The first focused run exposed a test-only teardown hang: dropping one Tokio duplex split writer did not signal EOF while its reader remained alive. The raw test peer now explicitly shuts down its writer before joining the server. That initial run was interrupted; a rerun passes. No production behavior was changed to conceal the test defect.
- The known task024 subprocess helper flake did not occur in the full-gate run. This worktree remains on the assigned base and does not incorporate the separate d2edb504 test-only correction; integration of that correction belongs to its coordinator. No production storage-lock change was made here.
- SPEC SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`; Cargo.lock still resolves Runtime 0.10.0 at `69faf9196991524b5549499d6208a1eaf567bf56`. No files outside the assigned transport/task paths changed.

## Coverage and limits

- Default production initialization supplies a working reload preparer without a setter. Other tests use the production HostResourceBuilder with explicitly fixture-sourced credentials and scheduling gates, never process-global environment mutation or a fake successful preparer.
- Typed, actual duplex stdio and real TLS WebSocket share acceptance and receipts: immutable returned snapshots, current direct-owner queries, independent duplicate operation IDs/ambiguous lookup, reservation/revision conflicts, read-only rejection without promotion, missing-preparer unavailability, real unknown-tool preparation failure, and all-or-nothing restart classification. Strict raw stdio/WSS requests reject absent fields, malformed IDs/revisions, extra authority/wait/private-key/preparer fields and the unsupported alias.
- A relay withholds only the LF from a genuine serve_stdio reload reply. A paused monotonic clock proves the original 40 ms deadline, single request and late-response consumption. Timeout and disconnection before complete receipt delivery leave accepted preparation/publication running, and later queries read the original owner record. A real WSS peer also disconnects during accepted preparation before reading any reply, then reconnects with the new key to query the original operation.
- Canonical publication tests distinguish changing a credential source name while keeping its actual key from actual key rotation. Unchanged keys preserve the original WSS connection and subscription; successful rotation ends old authenticated connections/subscriptions and releases only a network operator. Trusted typed/stdio operation rights survive, existing readers remain readers, and a newly authenticated connection follows ordinary first-arrival operation rights.
- A gated loopback HTTP/SSE provider proves an already accepted network-origin Runtime Turn finishes with resource A after canonical key rotation. The next Turn uses the actually reassembled provider B client and framework, retains its Session Role and real prior history, and the new-key WSS client reads the original reload receipt. These are real production provider/Runtime components talking to a local scripted endpoint, not live-provider validation.
- The transport layer adds no generic Operation entity, mutable business receipt cache, retries, replay/deduplication, preparer mutation, deep configuration merge, or defensive recovery loop. Owner saving/cleanup semantics remain the already reviewed core implementation.
- No external provider, real credentials, production CLI/process startup or TUI validation is claimed. Independent review and integration are coordinator work; no dev/main change or push was performed.

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
