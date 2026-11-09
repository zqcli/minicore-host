# Task 040: Connect manual compaction transports

## Scope and fixed inputs

- Worktree `/workspace/shared/minicore-host-compaction-transport`, branch `feat/compaction-transports`, integration base `21521919b5ecacea2fd72c698ea7521241a9e2d4`.
- Implementation ownership is `crates/host-transport/**` and this report only. No changes to core, protocol, resources, manifests, lockfile, root documentation or TUI are authored here. Required owner commits are applied as dependencies. No push, dev change or main merge.
- Unchanged SPEC v4.93 SHA-256: `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Runtime remains `69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0, with no path patch or dependency update. TUI reference remains `3beeb1d8a525e58ce6352e05061f242c5580e248` and is not tested or modified here.
- Requested worker selection was Astra with max reasoning; no separate model/runtime attestation is available, so no attestation is claimed.

Owner prerequisites were cherry-picked in order (original -> local hash):

- `ad5ec42c879515e7ad27b4148af6f1c25f05a146` -> `0e0ddc3fab5c7e330c69f4968a0ee1c12c0830ae`.
- `b03239b3936101ea95164314a1234917ba72ca82` -> `ddaede32edf5cc377320e442a48dec4053baa8e8`.
- `7171d8d436c00ce311ad4abe27520a6f779f4e39` -> `b2c282ec0ede2687f1e040b70ccb5e70875bbcc3`.
- `6893969eee546fd1469f236b3b596fa575c816ae` -> `5103d948aa53272b99ebd2e11eb69815ccd31414`.
- `d56b4496326480599d7b5062c2533aa329a30cdb` -> `f18d0a74b8e5f28af5a3062365e2342d0dbc5684`.
- Receipt-immutability review fix `3cfb84ce1ab0ceb9e57aff54fd257bc5185279fb` -> `8a836dc5ea43a4697f0c8322a3b464fea1ab1d3b`.
- Cancellation-fact validation review fix `270f5becc36294006a90dd30735525c3781b4089` -> `641673997c355e4437da137625ac3c36f2a26b7b`.

## Implementation and reference reuse

`SessionClient::compact` and `SessionClient::cancel_compact` use the existing typed Operation table, deadline, Host connection context and common stdio/WSS dispatcher. The only handlers call `Host::session_compact` and `Host::session_compact_cancel`; the transport adds no work owner, cache, queue, completion event, automatic retry or generic operation lifecycle. The supported route count is 37. Parameters carry the explicit SessionRef and canonical operation fields; cancellation has its own operation ID and target_operation_id. Strict malformed fields remain RPC errors and owner rejections remain business Faults.

Original CompactReceipt and CompactCancelReceipt facts stay immutable. Later progress comes from Session state and real owner events, or from a newly formed cancel receipt's target facts. Timeouts, aborted waiters and connection drops do not cancel accepted work. Permission checks remain in actual owner admission, not in a cached client capability.

Consulted minicore-agent code reference `f36a87366d8e2fe0a1c8216463887902e471c739`, especially `src/rpc/server/dispatch.rs` compaction branches. Its strict target parsing and delegation to the Session owner are useful reference patterns. Its completion-waiting RPC operation reservations, load wrapper and single operation-ID cancellation shape are not adopted: Host v4.93 requires immediate factual acceptance and an independent cancellation ID plus explicit original target. The mature shared Host operation/deadline/dispatch stack already fits this contract and is reused rather than adding a second stack. Agent is code reference only, not a compatibility or behavior baseline.

## Validation progress

- Nine new focused integration tests pass with real Host, Runtime, production resource assembly and provider HTTP/SSE on a local scripted endpoint. The first focused run had a test expectation error: public history reads append the genuine summary item, rather than remaining byte-identical. The corrected test verifies every original item is unchanged and exactly one real summary is appended. No production change masked that test error.
- Full workspace gate is running. Exact implementation/test hashes, final gate results and unique test inventory will be recorded after the source is fixed.
- Independent review and integration remain the coordinator's responsibility.

## Coverage and limits

- Typed, duplex stdio and TLS WebSocket verify real utility requests without fabricated Turns, summary publication, original receipt retention, explicit pause, both queue orders, unrelated Session progress and actual resumed model inputs.
- Exact cancellation preserves both queues and raw content before commit. Repeated and late cancellation cannot target a newer utility or Turn; wrong-Session and missing originals keep the actual Faults. A new connection operator can cancel an old operation, while an existing read-only observer is never implicitly promoted.
- Raw stdio/WSS frames reject missing/invalid target and operation IDs, extra authority/credential/timeout fields and the wrong cancellation shape. Transport IDs stay distinct from both business IDs.
- A relay delays only genuine Host response bytes. Real completion notifications can precede the original acceptance receipt. An incomplete response times out, a caller future can be aborted, and stdio/WSS can disconnect without cancelling or replaying owner work. Late replies do not close the shared connection.
- HTTP responses, localhost TLS certificates, credentials and response gates are explicit test fixtures. These are real production provider/Runtime paths against local scripted responses, not live-provider, production process, benchmark or TUI validation. No real secrets or public services are used.

All Rust commands use:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-compaction-transport-target
```
