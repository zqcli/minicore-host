# Exclude configured credentials from bash inheritance

## Scope and recovery baseline

- Branch: `task/066-recovered-credential-exclusion`.
- Recovered base: `7cc0bd2b399c3ea1320647e13d87bf57376934a5`, whose tree equals
  confirmed uploaded source checkpoint `ba1c10a7b15bad746761272d7b87a564dfb29efa`.
- This is not the lost `7261dfc1c9ed3fe1176a955e329e628cf5511dfe` / `b2915512`
  final tree. Earlier source, targets and evidence were lost when the execution
  environment was replaced. Old green results do not apply to this tree.
- Runtime remains pinned to `69faf9196991524b5549499d6208a1eaf567bf56`.
- Requested worker configuration: `gpt-6-astra`, `max`; effective runtime
  configuration cannot be independently attested from inside this task.
- No TUI, dependency, Cargo.lock, SPEC or remote change is part of this task.

Prevent direct child inheritance of every published model `api_key_env` and
optional network `access_key_env`, retaining formerly published names for this
Host lifetime. Preserve ordinary environment, real output and history. This is
not a process/filesystem sandbox or output-redaction mechanism.

## Red regression checkpoint

`crates/host-resources/tests/task066_credential_environment.rs` runs only known
synthetic environment inputs in a cleared subprocess with temporary HOME/TMPDIR.
It uses actual environment-based HostResourceBuilder preparation, persistent
Host/Agent/Session owners, Runtime, bash and a scripted loopback HTTP/SSE server.

The planned assertions cover initial selected and unselected model names,
network names without a listener, a deterministic gated old Turn across rename,
a subsequent Turn, removal of a model and network source, ordinary output and
the persisted native results after a real Host reopen. The gate prevents the
old Turn's next bash from starting until successful publication is observed.

On unchanged recovered production base, the regression exits 101 at its first
output assertion: both configured model names and the network source are still
present in bash; the ordinary value is retained. Later assertions are not yet
claimed passed. Evidence: `/workspace/shared/task066-recovered-evidence/baseline-persistent-resource.log`.
The recovered original memory-only test also reproduced the same red result on
this base; its separate log is `baseline-resource.log` in that evidence directory.

Commands use Rust/Cargo 1.90.0 and the coordinator's restored toolchain:

```sh
export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-target066
cargo fmt --all
cargo test -p minicore-host-resources --test task066_credential_environment --locked
```

Production implementation, core publication boundary tests and green validation
remain in progress. Independent task067 owns the app process-level regression.
