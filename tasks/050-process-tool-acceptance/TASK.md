# Task 050: Verify ordinary tools through the Host binary

## Fixed input and ownership

- Base: `f98af861d1bf618c99b8c28688ee1fb5122dacd5`, the composed executable,
  production ordinary-tool resources and canonical workspace routes.
- Worktree: `/workspace/shared/minicore-host-process-tools`, topic
  `test/host-process-tools`. Local commits only; no push or main merge.
- Owned additions: `crates/host-app/tests/ordinary_tools.rs`, its new
  `tool_support` module, and this record. Existing executable fixture support
  is reused unchanged. No production, manifest, lockfile, SPEC or TUI edits.
- SPEC v4.93 and Runtime 0.10.0 at
  `69faf9196991524b5549499d6208a1eaf567bf56` remain unchanged. This is actual
  binary acceptance, not a new implementation, live remote-model validation,
  TUI validation or a first-version completeness claim.

## Permanent local acceptance

Cargo supplies `CARGO_BIN_EXE_minicore-host`; each case spawns that real
executable. Configuration and credentials go through the normal application
entry point, and a persistent Agent/Session receives actual JSON-RPC over OS
stdin/stdout. A certificate-verified TLS WSS observer receives actual events
and performs read-only source and retention checks. No in-process Host, fake
Runtime, fake tool executor or success-prefilled result is used.

The new HTTP/SSE fixture gates real production-adapter requests individually,
including concurrent requests, and supplies only model responses. Tool results
are inspected in the next actual model request, owner queries, real file bytes
and durable native history. Gates, actual child output, TCP witness bytes,
kernel stopped state and child disappearance establish ordering. There are no
new sleep-based race guesses, ptrace/strace probes, retries or serial test runs.
The reused process fixture polls the actual exit status during teardown.

Four cases cover:

1. Actual write/read/edit, a failed edit and bash. CRLF, multibyte UTF-8 and
   final bytes are exact. Bash stdout/stderr, successful native outcome,
   process wait/cleanup facts and public completion positions agree. The tool
   is finished while the next model request remains gated. Tool edits retain
   their real Session/Turn/call identities; read and arbitrary shell writes do
   not become attributed edits. All six canonical workspace/change query
   methods run through stdio. Successful native results and exact JSONL bytes
   survive a clean binary restart; old observation sources remain explicitly
   unavailable rather than reconstructed.
2. A real Turn remains bound to the original root, tools and provider after
   Session update and Host reload. A new Turn in another Session binds the same
   canonical root through a symlink on the newly published resources. Both
   admitted Turns execute disjoint edits and their actual before/after records
   form a single complete chain, with no lost update. The next Turn uses the
   updated root/provider/tool selection. Workspace references reject a wrong
   or reconfigured root and accept the original canonical root via its alias;
   Session edit references never transfer to another Session.
3. Cancellation starts only after actual leader output and a descendant's TCP
   readiness witness. The fixture confirms the Host is OS-stopped, lets the
   real child write tail bytes into its pipes, receives a second TCP witness,
   queues `turn.cancel`, and resumes the Host. Both UTF-8 tails are retained
   exactly, no tail follows public tool completion, final positions match the
   complete bytes, and leader/descendant/group disappearance precedes observed
   Finished state and completion. The original cancelled result/outcome is
   persisted exactly once. A failure-only guard prevents stranded test children.
4. Host force starts only after real stdout/stderr and descendant readiness.
   Actual process exit follows group cleanup and persisted cancellation.
   An earlier successful write remains successful, its real file is retained,
   and both native results remain exact on binary reopen. Old Turns do not
   resume. The test does not require a shutdown receipt to reach the client,
   which is not an exit barrier in the specification.

## Scope limits and open aggregate gate

- Unix requires bash, Git and the existing executable/TLS support. The two
  cleanup cases are Linux-only and inspect `/proc` plus actual process-group
  absence. They cover an ordinary leader and inherited-group descendant, not
  escaped or hostile descendants or a cross-platform process matrix.
- Cancellation explicitly covers buffered tails. Force checks all bytes emitted
  before its deliberately held child gate, native outcomes and cleanup; it does
  not promise delivery of terminal observation events after process exit or
  create a permanent process-output archive.
- Shared binding is checked through observable simultaneous admission and the
  real edit chain across canonical aliases and old/new resource snapshots. It
  does not expose or inspect a private mutex identity or prove every possible
  concurrent schedule. Task 042's separate deterministic mutation-guard probes
  remain the component evidence for that internal invariant.
- Known diagnostics/storage fork-inherited lease blockers are not patched,
  serialized away, retried or declared fixed here. Full composed workspace
  validation and independent review remain separate parent gates after those
  corrections integrate. No whole-workspace success is implied by app tests.

## Validation record

Environment: Rustup/Cargo under `/workspace/shared`, Rust 1.90, two Cargo jobs,
debug information and incremental builds disabled, committed `--locked`
dependencies, `TMPDIR=/workspace/shared/minicore-host-tmp`, and private target
`/workspace/shared/minicore-host-process-tools-target`. No other target is
cleaned or reused.

- The existing executable target compiled successfully with
  `cargo test -p minicore-host --test process --locked --no-run`.
- New target compilation succeeded on its first compile. Initial fmt-check
  reported only the new files and was followed by normal formatting.
- First focused execution: 3 passed, 1 failed because the new test mistakenly
  requested nonexistent `host.reload.get`. The fixture now requests canonical
  `host.receipt.get`. No production path or expected business outcome changed.
  Original failure: `/workspace/shared/minicore-host-process-tools-initial-test.log`.
- Corrected focused run: all four cases passed. Log:
  `/workspace/shared/minicore-host-process-tools-focused.log`.
- The force case was then strengthened to preserve and read back a real write
  completed before cancellation. Final fixed checkpoint, lint, app inventory
  and validation commands/results are recorded below when complete.

## Final fixed-source validation

Test/support checkpoint:
`f74d793bb46f0e5ac274c0c102cd3d41d055985d`
(`test(host): verify ordinary tools through the real executable`).
The following documentation commit changes no tested source.

At that fixed checkpoint, all of the following passed:

- `./scripts/check-whitespace.sh`
- `cargo fmt --all -- --check`
- `cargo check -p minicore-host --all-targets --locked`
- `cargo clippy -p minicore-host --all-targets --locked -- -D warnings`
- `cargo test -p minicore-host --all-targets --locked`: **17 unique cases**, the
  four new ordinary-tool cases plus 13 unchanged executable cases. The binary
  unit-test target has no cases. These are package-scoped checks, not the
  blocked combined workspace gate.
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host --no-deps --locked`
- `git diff --check`

Gate log: `/workspace/shared/minicore-host-process-tools-final-f74d793.log`.
The separate canonical app inventory from
`cargo test -p minicore-host --all-targets --locked -- --list` confirms **17
unique target/test-name pairs**, not counts of subprocess invocations. Inventory:
`/workspace/shared/minicore-host-process-tools-inventory-f74d793.log`.

The normal parallel four-case target was then run ten times, stopping on any
failure, with `for iteration in $(seq 1 10); do cargo test -p minicore-host --test
ordinary_tools --locked; done`: **40/40 passed**. No sleep, serial runner,
retry-on-failure or assertion change was introduced. Repetition log:
`/workspace/shared/minicore-host-process-tools-repeat-f74d793.log`.

`cargo metadata --offline --locked --format-version 1` confirms exactly one
Runtime package, version 0.10.0 at the unchanged revision above. SPEC SHA-256 is
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` and Cargo.lock
SHA-256 is `151d9fb90f96072c9655f4f8a58ae4f4848f5caa2d1dc491fc2faa27ee2e79b8`.
Both match the fixed base. Production paths and existing app tests/support are
byte-identical to that base. No new production defect was observed in this
scope; independent acceptance review and the parent's complete workspace gate
after lease corrections remain outstanding. Whole-workspace tests, doctests
and inventory were intentionally not rerun against the known-blocked base.
