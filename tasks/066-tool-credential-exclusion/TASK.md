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

Red regression commit: `eb481bc` (`test(resources): reproduce configured credential inheritance`).

## Implementation and focused green checks

The configuration owner now holds one per-Host `ToolEnvironmentExclusions`
capability containing a deduplicated set of names only. Initial construction
seeds it from all configured models and the optional network source. Reload
extends it only inside the existing successful publication boundary, after
required save and credential validation. Preparing, rejected, conflicting and
unsaved candidates do not modify it. Removing or renaming a published source
does not remove its old name during this Host lifetime; independent Hosts have
independent policies.

`PreparedTurnBinding` exposes a read-only clone during local tool assembly.
The host-tools adapter forwards snapshots to local-tools. Bash copies names
immediately before command construction and applies `env_remove`; it never
holds an owner/policy lock across spawn or I/O. This filter-read boundary does
not retroactively alter an already-started child or make a concurrent spawn
atomic with reload. Model clients, selected tools, framework and workspace
bindings remain frozen for an old Turn. Ordinary environment and all actual
output/history remain unchanged.

Standalone `LocalTools::new` now requires an explicit `CommandEnvironment`.
Existing standalone tests choose `InheritCommandEnvironment`, which promises no
Host credential exclusion. There is no compatibility constructor, process-wide
environment mutation, Session traversal, redaction or new dependency.

Fresh recovered-tree checks passed:

- `cargo test -p minicore-host-core --lib task066 --locked`: 4 tests. Covers all
  configured names and deduplication, Host isolation, candidate construction,
  preparation failure, restart-required rejection, deterministic pending save,
  configuration conflict, successful publication and actual filesystem save
  failure. Logs: `core-tests.log`.
- `cargo test -p minicore-host-resources --test task066_credential_environment --locked`:
  1 test now passes through all assertions, including gated old Turn, next Turn,
  a third Turn after source removal, ordinary output, real JSONL and unchanged
  native results after Host reopen. Log: `resource-tests.log`.
- `cargo fmt --all -- --check`: passed.
- Strict Clippy over all targets in core, local-tools, tools and resources:
  passed (`--locked -- -D warnings`). Log: `clippy.log`.

Full relevant-crate tests/docs are being recorded next. Independent task067 owns
the separate app process-level authentication/output/history regression.
