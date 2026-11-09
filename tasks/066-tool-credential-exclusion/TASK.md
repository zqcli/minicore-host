# Exclude configured credentials from bash inheritance

## Scope and recovery baseline

- Branch: `task/066-recovered-credential-exclusion`.
- Recovered base: `7cc0bd2b399c3ea1320647e13d87bf57376934a5`, whose tree equals
  confirmed uploaded source checkpoint `ba1c10a7b15bad746761272d7b87a564dfb29efa`.
- This is not the lost `7261dfc1c9ed3fe1176a955e329e628cf5511dfe` / `b2915512`
  final tree. Earlier source, targets and evidence were lost when the execution
  environment was replaced. Old green results do not apply to this tree.
- Runtime remains pinned to `69faf9196991524b5549499d6208a1eaf567bf56`.
- The inherited TUI reference remains `3beeb1d8a525e58ce6352e05061f242c5580e248`,
  whose recorded Runtime lock is `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`.
  It is not this Host's Runtime revision and no TUI integration is claimed.
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

Production commit: `0abe2dfdd8e0583724f73e150ed11feb16021042`
(`fix(tools): exclude published credential environment names`).

## Completed scoped validation

All commands below exited 0 on the recovered task066 production tree. The
associated files are in `/workspace/shared/task066-recovered-evidence/`.

```sh
cargo fmt --all -- --check
cargo clippy -p minicore-host-core -p minicore-host-local-tools -p minicore-host-tools -p minicore-host-resources --all-targets --locked -- -D warnings
cargo test -p minicore-host-core -p minicore-host-local-tools -p minicore-host-tools -p minicore-host-resources --all-targets --locked
cargo test -p minicore-host-core -p minicore-host-local-tools -p minicore-host-tools -p minicore-host-resources --doc --locked
RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core -p minicore-host-local-tools -p minicore-host-tools -p minicore-host-resources --no-deps --locked
cargo test -p minicore-host-core -p minicore-host-local-tools -p minicore-host-tools -p minicore-host-resources --all-targets --locked -- --list
./scripts/check-whitespace.sh
```

- All **417 unique tests** across these four affected crates pass, including the
  five new tests. The count comes from `test-list.log`, not summed nested probe
  output. The isolated resource subprocess is one test, not another test case.
- Doctests pass with zero cases in these crates; strict documentation build
  passes. Logs: `doctests.log` and `docs.log`.
- `fmt.log`, `clippy.log`, `relevant-tests.log` and `whitespace.log` record the
  remaining gates. `git diff --check` also passes.
- SPEC SHA-256 remains
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
  All manifests and Cargo.lock are unchanged. Dedicated target directory used
  about 1.3 GiB after validation, with more than 50 GiB disk free.

These are scoped fresh checks, not a reconstructed old green, a full-workspace
integrated gate, a live provider test, or a TUI/WSS transport test. Task066 checks
the real access-key owner and HTTP/SSE provider against isolated fixtures.
Independent task067 owns the separate actual app process-level WSS/provider
authentication and output/history regression. Independent review and the
coordinator's recovered-tree integration gate remain separate. No remote was
written and no claim is made about arbitrary same-user file/process access.
