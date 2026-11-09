# Regress configured credential inheritance through the binary

## Scope and exact sources

- Branch: `task/067-recovered-process-regression`.
- Fixed production base: `583d3c08fb3722d7d80af3bddb91d8d33253acb7`.
  This includes the recovered source prefix, task070 query corrections and
  task066 credential exclusion; it is not the lost H7261dfc tree.
- Fresh red production base: `7cc0bd2b399c3ea1320647e13d87bf57376934a5`.
- Requested worker configuration: `gpt-6-astra`, `max`; effective runtime
  settings cannot be independently attested from inside this task.
- Adds only `crates/host-app/tests/credential_environment.rs` and this task note.
  Existing support, production, public documentation, manifests, Cargo.lock,
  dependency pins, SPEC and TUI are unchanged. No remote write is performed.

The single Unix integration case starts the actual `minicore-host` binary with
`env_clear`, PATH `/bin:/usr/bin`, a temporary empty HOME/TMPDIR, LANG, two
synthetic configured credential values and one ordinary synthetic value. It
never enumerates ambient environment or reads /proc or real user files. Existing
Process, local HTTP/SSE, TLS/WSS and history support is reused. The new test reads
the current `TurnListSnapshot.data` directly instead of the restored shared
`only_turn` helper's obsolete bare-array assumption; shared support is not edited.

The local provider verifies the production HTTP Authorization header. Verified
TLS WSS authentication succeeds before execution and after a real binary reopen.
Only model replies are scripted. Bash prints three explicit environment names,
not supplied credential values or fabricated tool output. The test requires:

- Real process exit/wait/cleanup and empty stderr.
- Exact agreement between actual WSS output, native ToolResult in the next HTTP
  request body, `toolcall.get`, retained stdout and public history.
- A completed and persisted Turn; actual native JSONL output matches the public
  result, and an actual Host reopen preserves public history and JSONL bytes.
- Both configured names are absent, rather than merely assigned empty values;
  the unrelated ordinary value remains inherited.
- No synthetic credential value in configuration, native results, next request
  body, public tool/history queries or JSONL. TRACE diagnostics are also checked.

The exclusion assertion is deliberately after the persistence/reopen assertions,
so a security-red run demonstrates the complete real propagation path first.
No output rewriting, generic secret detector, sandbox or live-provider claim is
part of this regression. Task066 separately owns all configured-name coverage,
successful gated same-Turn reload, rename/removal and failed-publication checks.

## Reproducible commands and evidence

Rust/Cargo 1.90.0, locked dependencies and independent red/fixed target directories:

```sh
export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0

# In /workspace/shared/minicore-host-credential-regression-red at 7cc0bd2:
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-target-credential-process-067
cargo test -p minicore-host --test credential_environment --locked \
  binary_keeps_configured_credentials_out_of_tool_output_and_persistent_history \
  -- --exact --nocapture

# In /workspace/shared/minicore-host-credential-regression at 583d3c0:
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-target-credential-process-067-fixed
cargo test -p minicore-host --test credential_environment --locked \
  binary_keeps_configured_credentials_out_of_tool_output_and_persistent_history \
  -- --exact --nocapture
cargo fmt --all -- --check
cargo clippy -p minicore-host --test credential_environment --locked -- -D warnings
cargo test -p minicore-host --test credential_environment --locked -- --list
./scripts/check-whitespace.sh
```

Evidence directory: `/workspace/shared/task067-recovered-process-evidence`.
`red-source.txt` and `green-source.txt` record exact sources, toolchain and hashes;
`red.log` and `green.log` retain complete actual command output and exit status.

Fresh red exited 101 at the final stdout exclusion assertion, after all real
execution, public-query, persistence and reopen checks passed. Actual stdout was:

```text
model=app-fixture-only-model-credential
host=app-fixture-only-network-credential
ordinary=fixture-only-ordinary-environment
```

The same test source is used for red and fixed validation, SHA-256
`1bd7c4ad12b010ab28ab02027201473ddd8ea640cd805bb4a45f3038eed6887b`.
Recovered Cargo.lock remains
`151d9fb90f96072c9655f4f8a58ae4f4848f5caa2d1dc491fc2faa27ee2e79b8`;
SPEC remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

Fresh fixed validation passed: one case, zero failures, exit 0. The new target
actually compiled the fixed worktree before running the binary. Binary SHA-256:

- Red: `687aa5891f363529c79801058e319c20607c52408bcdd4e47bc027a9f8033318`.
- Fixed: `cc4d86f47210e7ef813c0e8f1d732c6b533e92da431c92db8d1802e1ce13f709`.

Formatting, strict focused Clippy and whitespace checks passed. The focused
`--list` command reports exactly one test and zero benchmarks. Their logs are
`fmt.log`, `clippy.log`, `whitespace.log` and `test-list.log` in the same evidence
directory. The production and test sources did not change after these checks.

## Recovery and validation limits

The earlier H7261dfc red output recovered under `/workspace/shared/recovery-067`
is historical evidence only, not a rerun or a result for the recovered tree.
An initial restored-helper failure is retained separately as
`recovered-helper-preflight.log`; it is not the security red. An initial attempt
to reuse the red target from another worktree executed the identical red binary
without recompiling; `reused-target-unverified.log` is not fixed validation. The
fixed run above uses a newly created, separate target directory.

This focused test is one unique case. The coordinator owns the final integrated
workspace lint/test/doc gate and independent review. No full-workspace or live
provider/TUI validation is claimed here.
