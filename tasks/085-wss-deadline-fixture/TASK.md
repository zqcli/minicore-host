# Make the WSS initialization deadline fixture deterministic

## Scope and provenance

Base: `101abcb25a416a81f8350ca1acdbe6078b0de0bc`; topic
`task/085-wss-deadline-fixture`; isolated worktree
`/workspace/shared/minicore-host-task085`, with its own `target` directory.
Requested worker configuration: Astra/max; effective runtime settings are not
independently attested. The parent owns PROJECT.md, independent review and
aggregate integration. No push.

Only the initialization deadline test in
`crates/host-transport/tests/wss_wait.rs` and this record change. There is no
new helper, production change, timeout increase, error suppression, global
concurrency change or dependency change. SPEC v4.93, all manifests, Cargo.lock,
Runtime `69faf9196991524b5549499d6208a1eaf567bf56` (0.10.0) and TUI stay unchanged.

## Failure and deterministic proof

CI run `42737165717`, job `129569097812`, checked out
`138d0814504c25503704f4c7739d52a0b50d1cd0` (checkout log lines 100 and 112).
Its `wss_wait` test failed at line 179 when the peer unwrapped TLS acceptance:
`UnexpectedEof("tls handshake eof")`. The raw log is
`/workspace/shared/publication-fb2acae/ci-job-129569097812.log`, SHA-256
`73cb4771b4ca772425ee2714095363f8e1f112f7d94fe6247cbb5d351afc7149`.
The CI commit object is not in this local checkout; a direct Git source diff
against that commit was therefore not available.

The base fixture uses an overall 200 ms client budget, a 100 ms wall-clock
sleep before TLS and a further 120 ms sleep after authentication. Scheduling
and real handshake work consume the same budget. The test cannot require TLS
to succeed if the client has already legitimately expired its original wait.

Fresh evidence is in `/workspace/shared/recovery-second/task085/`:

- `red-pre-tls-proof.rs` replaces only this test temporarily on the fixed base.
  It uses the real client and accepted TCP socket, pauses Tokio time, advances
  to 200 ms before the server attempts TLS, and first asserts the actual
  `CallError::WaitTimeout { wait_timeout_ms: 200 }`. The original unconditional
  TLS unwrap then reproduces the exact `UnexpectedEof("tls handshake eof")`.
  This controlled schedule proves the failure mechanism; it does not claim
  that an unmodified local run happened to reproduce the CI scheduling.
- `red-pre-tls-proof.log` records the timeout assertion reaching its diagnostic
  and the TLS panic; exit 101 is in `red-pre-tls-proof.exit`. Probe source
  SHA-256: `5829351d781650f7eb7de1db5af10d7529e13a84f0996608cf1d2d73e30e623a`.
  The red probe is not shipped as a test or helper.
- `base-wss_wait.rs` preserves the unmodified base fixture. `env.sh` records the
  existing Rust homes, this worktree's dedicated target, two build jobs,
  disabled incremental compilation and zero dev/test debug info.

## Correction and contract

SPEC chapter 10 requires one monotonic wait from call initiation through the
complete response, with no layer restarting the budget, and preserves timeout
versus connection/protocol error provenance. Production already wraps the
entire real TCP/TLS/WebSocket/authentication path with that deadline.

The test now starts paused and polls the public `HostClient::wss` future before
advancing any time. It drives the real client and peer together, advancing
60 ms after actual TCP acceptance and 60 ms after actual TLS acceptance. The
peer completes the real WebSocket handshake, reads `initialize`, and validates
the actual supplied key using `Host::connect_network`.

A local `select!`/`yield_now` branch prevents Tokio from automatically jumping
to the timeout while loopback I/O is pending. It does not establish ordering
by sleeping or modify global concurrency. The authenticated socket and Host
handle remain alive without a response. The client is explicitly polled and
must remain pending at 199 ms, then return precisely `WaitTimeout(200)` at an
elapsed 200 ms. A 201 ms test guard rejects a later restarted deadline rather
than waiting for it; 201 ms is not an allowed client budget, and the exact
200 ms elapsed assertion remains mandatory. No late socket error is accepted
as a substitute for the wait timeout.

This is a transport-clock fixture using real TCP, TLS, WebSocket and Host
authentication, not the production WSS server or a Runtime-turn test. The
unchanged real-Runtime WSS case remains separate.

## Validation

Run from the isolated worktree after sourcing the evidence `env.sh`. All Cargo
dependency commands use `--locked --offline`.

- `cargo test -p minicore-host-transport --test wss_wait
  initialization_budget_covers_tls_websocket_and_authenticated_response_once
  --locked --offline -- --exact --nocapture`: the controlled red proof fails as
  above; the corrected test passes (`green-deadline.log`).
- The Cargo-produced `wss_wait` executable, resolved by
  `cargo test -p minicore-host-transport --test wss_wait --no-run --locked
  --offline --message-format=json`, passes 100/100 exact-test repeats and
  25/25 complete five-case suite repeats, with default test concurrency.
  See `test-artifacts.jsonl`, `wss-wait-binary.txt`,
  `repeat-deadline-100.log` and `repeat-wss-wait-25.log`.
- `cargo test -p minicore-host-transport --all-targets --locked --offline`
  passes all 143 scoped cases (`transport-all-targets.log`).
- `cargo fmt --all -- --check` passes (`fmt.log`).
- `cargo clippy -p minicore-host-transport --all-targets --locked --offline --
  -D warnings` passes (`transport-clippy.log`).
- `cargo test -p minicore-host-transport --doc --locked --offline` passes its
  one compile doctest (`transport-doctest.log`).
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-transport --no-deps
  --locked --offline` passes (`transport-doc.log`).
- `cargo test -p minicore-host-transport --all-targets --locked --offline --
  --list` lists 143 transport cases (`transport-test-list.log`), not a
  workspace-wide test count. `./scripts/check-whitespace.sh` and
  `git diff --check` pass.
- Linux-filtered offline Cargo metadata confirms only the pinned Runtime
  revision/version (`metadata-linux.json`, `runtime-and-count.txt`). An
  unfiltered metadata attempt could not inspect uncached cross-platform
  dependency `bumpalo v3.20.3` offline; no dependency or lockfile was changed.
- Protected-file checks pass. SPEC SHA-256 stays
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`;
  Cargo.lock SHA-256 stays
  `1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`.

Independent review and the workspace-wide aggregate gate belong to the
parent. Live-provider, full-stage, performance and TUI acceptance are not
claimed by this test-fixture task.
