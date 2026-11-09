# Canonical direct-owner controls

## Scope and baseline

- Baseline: `413916cac0730d291e5c8163cdc3d59e5dbedcd1`, unchanged SPEC.md v4.93.
- Runtime stays pinned to published `18a6ac9b82d003330cd870275efca793cdcc9a96`.
- Implement canonical requests/receipts for existing Agent stop, Session pause/resume/load/unload/close/cancel and exact Turn cancellation. Retain formation-time facts with their actual owners for Host receipt lookup.
- Preserve current authority checks, exact execution identity, save continuation ownership, abandoned-wait behavior, and normal memory reclamation. Do not add Operation entities, deduplication, replay, retries, caches of released bodies, or unsupported method handlers.
- Root manifests, project documents, provider/storage and TUI are outside this worktree's edit scope. This increment does not implement all 52 methods, network authentication, WSS, or TUI.

## Checkpoints

1. Protocol facts (`881624b`, `feat(protocol): define canonical receipt lookup variants`): `SessionCancelReceipt` describes an actually observed current Turn or an idle cancellation, with validated optional identity/state pairing. `Receipt` serializes the original concrete kind and operation ID, without a generic envelope.
   - `cargo test -p minicore-host-protocol --test control_catalog --locked`: passed 15 tests, including idle/active cancellation pairing and unknown-kind rejection.
2. Direct-owner implementation (`f10d859`, `feat(core): form control receipts at direct-owner acceptance`): canonical Host entry methods delegate to Agent/Session controls. Formation and owner retention occur within the same acceptance/state lock as the mutation; shutdown uses the same boundary. Each repeated correlation ID retains distinct invocation facts and singular lookup refuses ambiguity. Lookup excludes internal Session-creation steps belonging to one Agent creation. Session receipts are discarded on actual reclamation, and an immediately reclaimed close still returns its accepted receipt.
   - Snapshots and controls share typed PauseReason/BlockReason facts. BlockReason retains the complete original Fault, with an explicit release condition; metadata/history save fault fields keep their separate meanings.
   - Load/resume return actual loading and pause facts. The original asynchronous loader and save continuation remain owned after caller revocation; a later explicit pause wins. Legacy controls share these primitives. Query DTOs additionally derive Deserialize for the separately implemented typed stdio client.
   - `cargo clippy -p minicore-host-core -p minicore-host-protocol --all-targets --locked -- -D warnings`: passed.
   - `cargo test -p minicore-host-core -p minicore-host-protocol --locked`: passed 156 tests with the eight new owner regressions present in the working tree; final regression commit and workspace gates follow.

## Validation environment

All Cargo commands use Rust 1.90 and:

```
RUSTUP_HOME=/workspace/shared/rustup
CARGO_HOME=/workspace/shared/cargo
PATH=/workspace/shared/cargo/bin:$PATH
TMPDIR=/workspace/shared/minicore-host-tmp
CARGO_TARGET_DIR=/workspace/shared/minicore-host-controls-target
```

Intermediate baseline checks after refactoring:

- `cargo check -p minicore-host-core -p minicore-host-protocol --all-targets --locked`: passed.
- `cargo test -p minicore-host-core -p minicore-host-protocol --locked`: passed the pre-existing 147 unit/integration tests (100 core, 47 protocol); zero doc tests in these two crates.
- Initial compile iterations exposed private Fault field access, changed internal test signatures, and one borrow overlap; fixed before the passing checkpoint above.
- `cargo test -p minicore-host-core --test runtime_owners canonical --locked`: passed eight new owner/Runtime fixture regressions. The complete final gate passed below.

Tests use actual Host/Agent/Session owners and pinned Runtime with explicit model fixtures and real storage files where applicable, not a live model provider or TUI.

## Final regression checkpoint

The `8a68edb` (`test(core): verify canonical control acceptance and retention`) checkpoint adds 12 core regressions (eight Runtime-owner tests and four persistent-owner tests), plus the protocol regression from `881624b`. Existing accepted-creation and original-save tests now also assert canonical lookup facts.

Coverage includes immutable load/pause/unload/resume receipts; unload-specific pause reasons; duplicate operation IDs within and across owners; precise T1 cancellation after T2 starts; no repeated inbox clear; idle/current Session cancellation; Agent stop preserving queued work and reporting actual stopping/stopped facts; immediate memory close returning its accepted receipt despite reclamation; persistent closing facts remaining unchanged after closed; authority changes and observation; graceful-to-force shutdown facts; public creation lookup excluding nested creation steps; and real asynchronous load/save completion after caller revocation without rewrites or reruns.

Final `./scripts/check.sh` passed with the environment above:

- `./scripts/check-whitespace.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --all-targets --locked`: 253 unique unit/integration tests passed (112 core, 48 protocol, 36 provider, 57 storage). The storage subprocess probe also prints its own nested one-test result; it is not counted twice.
- `cargo test --workspace --doc --locked`: two doc tests passed.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`: passed.
- `sha256sum SPEC.md`: still `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

One earlier full gate passed 252+2 tests before adding the automatic-history-loading receipt regression. The final gate above includes it. Production source has not changed after `f10d859`.

## Handoff and limits

Ready for independent review and parent integration of the local commits. The canonical signatures and protocol variants were sent directly to the isolated typed/stdio adapter worker. No root manifests, README, PROJECT.md, other documentation, provider/storage implementation, Runtime pin, or TUI changed. No remote push or main merge was authorized or performed.

Receipt lookup reports only retained owner facts. Reclaimed Session controls can become unavailable immediately after a successful close; the returned close receipt remains valid. Ambiguous reused IDs return an explicit lookup error; `Host::receipts` exposes each retained invocation separately. These are correlation queries, not retry or replay mechanisms. Deferred recovery architectures remain out of scope.

## Integrated lookup-fault correction

Parent integration maps `881624b` → `19008cb`, `f10d859` → `7197366`,
`8a68edb` → `12efa6a`, and `fcf3580` → `deb14d8`.
Independent fixed-snapshot review passed all 160 core/protocol cases but confirmed
P2: missing retained receipts and ambiguous reused operation IDs both used 1401,
the same Fault as an unimplemented method. SPEC requires type/code, not prose,
to distinguish handling semantics.

The correction registers two narrow HostLookup leaves: 1202
`HOST_RECEIPT_NOT_FOUND` and 1203 `HOST_RECEIPT_AMBIGUOUS`. Canonical lookup and the
existing creation-only helpers share those meanings; 1401 retains its method
availability meaning. No new error framework, retry behavior or object retention
is added. Existing collision/reclamation checks now assert exact Fault kinds;
a new real-owner regression compares missing, one match, multiple independent
matches and genuinely unsupported method behavior. The protocol registry's
round-trip/uniqueness checks include the two leaves.

`cargo test -p minicore-host-core -p minicore-host-protocol --locked` passed 161
tests (113 core +48 protocol) after the correction; formatting and diff checks
also passed. The complete integrated gate and fixed correction review follow
separately; this focused result is not yet a full-gate claim.

The integrated correction is `b961f9f45ea31def2e215fa07368c05893cdf6fa`.
`./scripts/check.sh` passed on that exact commit: 254 unit/integration tests
(113 core, 48 protocol, 36 provider, 57 storage), 2 doctests, formatting, strict
all-target Clippy/rustdoc and committed whitespace. Log:
`minicore-host-validation/canonical-integrated.log`. Independent correction
verification is in progress; the earlier P2 is not called reviewed merely because
this gate passed.
