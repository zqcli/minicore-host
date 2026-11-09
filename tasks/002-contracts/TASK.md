# Implement Public Contracts

Status: completed for the assigned protocol-foundation scope. Root workspace integration belongs to the parent task. Scope: `crates/host-protocol/**` and this task file only.

Baseline: SPEC v4.93, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` verified unchanged.

## Milestone 1: identity, ownership, configuration, faults

- Strong random 20-symbol IDs use the exact alphabet and OS CSPRNG. Provider ToolCall IDs remain unchanged and Turn-scoped.
- Flat direct-owner references, storage-mode matrix, lifecycle/outcome/persistence values.
- Strict complete configuration, explicit create/update patch semantics, nullable workspace clearing, immutable candidate construction and owner-provided resource validation. No application quota fields.
- Existing Runtime reference grammar and retry/deadline ranges retained explicitly; Role is not a permission or execution identity.
- Single fault registry preserves all SPEC codes and adds named owner-boundary faults; strict type/code/details/retry validation and distinct client-side errors.
- Exact dependency pins match the Runtime reference lock: getrandom 0.4.3, serde 1.0.229, serde_json 1.0.154. Temporary standalone crate workspace/lock; parent integrates the final root Cargo.lock.

Validation (Rust 1.90.0, workspace-local official toolchain):

```sh
export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo PATH=/workspace/shared/cargo/bin:$PATH
cargo fmt --manifest-path crates/host-protocol/Cargo.toml
cargo test --manifest-path crates/host-protocol/Cargo.toml
cargo clippy --manifest-path crates/host-protocol/Cargo.toml --all-targets -- -D warnings
```

Tests: 1 internal uniform-encoding test and 13 integration tests passed. Clippy initially found a derivable Default implementation; fixed before the milestone commit and rerun. No remote writes or provider/TUI claims.

## Subsequent milestones (completed below)

Strict text requests and typed receipts, connection/control facts, exact 52-method catalog with explicit staged availability, additional positive and negative wire tests.

## Milestone 2: text requests and receipt facts

- Milestone 1 commit: `a87e35978c20d8b7fa2339345bf7ee77f75bce8c`.
- Text-only MessageInput/SendMessage rejects source/authority/attachment/wait options; direct-owner message targets and trusted historical provenance are separate.
- Typed message.send acceptance retains actual queue and optional known startup Fault without asserting execution or persistence.
- Agent/Session creation requests and complete/partial/progress receipts enforce publication IDs, requested-message presence, prerequisite order and acceptance facts in both serialization directions. Ordinary create-call return validation is distinct from pending/running progress queries.
- Added 12 positive/negative message/creation tests. Initial test exposed serde's unknown-field handling for tagged unit variants; CreateStep now has explicit strict wire validation. Final 26 total tests and clippy passed using the commands above.

## Milestone 3: control facts and canonical catalog

- Milestone 2 commit: `83f9a575a75ce11870a86d6ce4f016cbb53b8f66`.
- ConnectionInfo and shared control state retain distinct meanings; explicit positive CallOptions defaults to 30000 ms and remains caller-side.
- Shutdown/control requests and receipts enforce force-versus-draining, actual cancellation/close progress, effective pause and consistent residency facts. Manual compaction request association is represented without inventing a new operation lifecycle or unfinished result schema.
- Canonical 52-method enum/catalog is tested against SPEC §8 in exact order; real services supply explicit implemented-method sets. All other methods remain staged and unavailable, with no fake-success handlers.
- Typed MessageClient/MessageService templates separate request, trusted owner context and waiting options.
- Configuration serialization validates direct Rust candidates; strict provenance rejects user/Agent-source contradictions, including tagged-unit serde edge cases.
- 39 total tests and clippy passed. First-version endpoints, transports, live Runtime and TUI are not claimed by contract-only coverage.


## Milestone 4: RPC mapping and final verification

- Milestone 3 commit: `dfc62eeab02692cfaa1228b8bcfa1a37a286cb84`.
- Final implementation commit: `323975774e22b14a5fc46806ca376b17ee3bd020`.
- MessageSendRpcRequest and RpcResponse enforce the same local business values, separate transport IDs and business IDs, require JSON-RPC 2.0, keep business Fault codes separate from protocol codes, and reject simultaneous result/error. Accepted plus start_error remains a normal result.
- Fault registry now also owns allowed details schemas and retry-hint eligibility. Explicit null is rejected for absent-only optional fault fields; snapshot no-block and unidentified protocol-error IDs have explicit null encoding.
- Crate README documents integer encoding, exact configuration fields, resource validation, caller/control separation, staging and remaining adapter work. Unknown future Fault preservation in clients remains an adapter responsibility; strict service Fault types do not accept arbitrary unknown leaves.
- A temporary build error in the new required-nullable helper was fixed by fully qualifying Deserialize; final tests/lint/docs were rerun after that fix. No runtime workaround or failing check is hidden.

Final validation on the final source with Rust 1.90.0:

```sh
export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo PATH=/workspace/shared/cargo/bin:$PATH
cargo fmt --manifest-path crates/host-protocol/Cargo.toml -- --check
cargo test --manifest-path crates/host-protocol/Cargo.toml
cargo clippy --manifest-path crates/host-protocol/Cargo.toml --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --manifest-path crates/host-protocol/Cargo.toml --no-deps
git diff --check
sha256sum SPEC.md
```

Outcomes: 47 tests passed (1 internal, 15 foundation, 12 message/creation, 14 control/catalog, 5 RPC); zero failures; Clippy with denied warnings passed; rustdoc with denied warnings passed; formatting and diff checks passed. SPEC hash remains the unchanged baseline. No provider credentials or live calls were used. No remote push occurred.

## Integration and remaining scope

Cherry-pick the four implementation commits in order: `a87e35978c20d8b7fa2339345bf7ee77f75bce8c`, `83f9a575a75ce11870a86d6ce4f016cbb53b8f66`, `dfc62eeab02692cfaa1228b8bcfa1a37a286cb84`, `323975774e22b14a5fc46806ca376b17ee3bd020`, followed by this task-log commit. Parent removes the temporary crate `[workspace]`, integrates exact dependencies into the committed root lock, and reruns complete workspace checks. Root manifests, PROJECT.md and other crates were not edited here.

The contract catalog does not implement all 52 services. Session compact result phases, auxiliary query payloads, typed receipts for not-yet-integrated controls, event/read synchronization, trusted CallerContext registration, real client wait ownership and unknown-leaf preservation, WSS/stdio handlers and TUI acceptance remain with their implementation stages. These foundation tests do not certify those behaviors. Independent review is still required by the repository workflow.
