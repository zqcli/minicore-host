# Task 110 — Manual compaction tool ordering (0.11.0 dev)

## Scope and baseline

- Owner request: fix daily acceptance REPORT F2 in the isolated
  `fix/0110-compaction` worktree; no local Cargo/builds, publication, version
  changes, TUI source changes or dependency-pin changes.
- Host baseline: `32f074c57868533af81b0c6491440bae24b872c8` (0.11.0).
- Report: `/Users/zzq/Develops/minicore-remote-build/acceptance-0110-daily/REPORT.md`.
  Its TUI baseline is `bd6f9ed3d108b9594a7f75d3f2c491bfed898520` (0.11.0),
  with that same Host pin; neither is changed here.
- SPEC 4.10.2 SHA-256:
  `55476e6d2c6c9ec5b857f2b9235834a162b3ad85ce56a8e2a295135f7dc370c7`.
  Sections 3/5 and acceptance require legal resource selection, Session-owned
  compaction, no tool execution authority/extra Turn, unchanged original
  history, and shared Runtime request validation and utility safety.
- Imported Runtime baseline remains `3aa47629929906832dcb309afb7d242c1ae3e53f`
  as recorded in Task 106; no Runtime code or dependency is changed.
- Requested engineering worker configuration: `gpt-6-astra`, `max` reasoning;
  effective runtime settings are not independently attested. No subagent tool
  is available in this worker session.

## Implementation

Implementation commit: `8fb386d6ee070d9edf291fde32b8ed57f099ff38`
(`fix(resources): normalize manual compaction tool declarations`).

`crates/host-resources/src/resolver.rs` now sorts collected compaction tool
schemas by name, matching ordinary ToolSet ordering without binding executable
tools. Validation still runs first; there is no deduplication or filtering of
unknown selections. Runtime's strictly ordered request check is unchanged.

The production resolver regression compares complete schemas for three orders
of the same five builtins, constructs a valid Runtime request, and verifies
rejection of duplicate and unknown tools. The Responses regression exercises
real Host/Session/Runtime/provider composition with the default
`read, write, edit, apply_patch, bash` order: two ordinary Turns, manual
compaction with a tool-free utility request, preserved raw history, no added
Turn, and a subsequent completed Turn using the summary and selected tools.
The loopback SSE responses are explicitly scripted fixtures, not live-provider
or TUI acceptance. Existing empty-tool, Chat and automatic-compaction tests
remain in scope for regression validation.

## Validation

Remote root: `/root/mct/fixes-0110/host/src` on `root@192.168.20.199`, via SSH
socket `/tmp/minicore-fixes-0110.ssh`. Sources are rsynced excluding `.git` and
`target`. All Cargo commands use `flock -w 1800
/root/mct/fixes-0110/cargo-host.lock`; dependency-resolving commands use
`--locked`. Environment: `PATH=/root/.cargo/bin:/usr/lib/llvm-19/bin:$PATH`,
`CARGO_TARGET_DIR=/root/mct/fixes-0110/target-host`. Remote rustfmt output is
copied back only for this task's Rust files.

Final gates use Rust/Cargo 1.90.0 (`RUSTUP_TOOLCHAIN=1.90.0`) on Linux x86_64.
The incomplete remote toolchain was repaired before valid red/green evidence;
earlier missing-standard-library and fixture compile errors are not counted as
regression failures. The initial stable-toolchain compile attempt is also not a
quality-gate pass. No lint suppression or dependency updates were introduced.

Removing only the production sort (retaining the new tests) produced two actual
failures, each exit 101:

- `cargo test -p minicore-host-resources --lib --locked production_compaction_resolver -- --nocapture`:
  default-order declarations differ from canonical order.
- `cargo test -p minicore-host-resources --test responses --locked manual_compaction_with_default_tool_order -- --nocapture`:
  the actual manual operation finishes Failed with
  `model request tools are invalid or not strictly ordered` and no utility call.

The original fix was restored before green validation. Remote logs are retained
at `/root/mct/fixes-0110/host/logs/` and copied to
`/tmp/minicore-compaction-evidence/` locally (not added to the repository).
All final gates passed on 2026-11-14:

| Command (Cargo commands under the lock above) | Result |
| --- | --- |
| `cargo fmt --all`, then `cargo fmt --all -- --check` | Pass; formatted files copied back and SHA-256 matched |
| `cargo test -p minicore-host-resources --locked` | 56 tests passed, including both new regressions |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `setpriv --bounding-set=-dac_override,-dac_read_search cargo test --workspace --all-targets --locked` | Pass, no ignored tests |
| `cargo test --workspace --all-targets --locked -- --list` | 1,180 unique target/test pairs in 141 targets; nested subprocess output not counted |
| `cargo test --workspace --doc --locked` | 7 doctests passed |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` | Pass |
| `PYTHONDONTWRITEBYTECODE=1 python3 scripts/task059_projection_test.py` | 15 passed |
| `PYTHONDONTWRITEBYTECODE=1 python3 scripts/task058_rss_test.py` | 5 passed |
| `PYTHONDONTWRITEBYTECODE=1 python3 scripts/task090_perf_test.py` | 8 passed |

The first unrestricted-root workspace run hung in the unchanged
`owner_diagnostics/lifecycle.rs:255` permission fixture. An isolated `--nocapture`
run proves that `unwrap_err()` receives `Ok(())` when root writes to its chmod-500
directory, then teardown waits on the unreleased fixture gate (timeout exit 124).
The hung full-run test process was explicitly terminated. Dropping only
`CAP_DAC_OVERRIDE` and `CAP_DAC_READ_SEARCH` for the test process restores the
intended filesystem denial; the isolated fixture and full suite then pass.
No test code, assertion or production behavior was changed to accommodate this.

Local `scripts/check-whitespace.sh` cannot run under macOS Bash 3 because it
uses `mapfile`. Its exact checks were instead run directly: non-shallow history,
one root commit, `git diff --check <root> HEAD`, `git diff --check --cached`, and
`git diff --check`; all passed. New project/task links were checked locally.
No Cargo command or compilation ran locally. No native macOS/Windows execution,
actual standard-user account run, release packaging, live provider or TUI
acceptance is claimed.

## TUI local-source build recommendation

Do not publish Host merely to make Cargo resolve this fix, and do not change the
TUI repository's committed manifest or pin. Use an isolated copy of the parent's
integrated TUI sources, a temporary Cargo config with
`[patch."https://github.com/zqcli/minicore-host"]` mapping all **12 Host library
packages** to the integrated local Host `crates/*` paths, and a scratch lockfile.
In the scratch lockfile only, remove those 12 Host Git `source` fields; retain
all package versions, external sources, checksums and dependency lists.

This resolution strategy was checked remotely with the unchanged TUI baseline
in `/root/mct/fixes-0110/host/tui-check`, config
`/root/mct/fixes-0110/host/patch-overrides.toml`, and:

```sh
cargo metadata --format-version 1 --filter-platform x86_64-unknown-linux-gnu \
  --locked --offline --config /root/mct/fixes-0110/host/patch-overrides.toml
```

Exit 0; all 12 resolved Host packages have version 0.11.0, no Git source and
manifest paths in this local Host tree. A parsed lockfile comparison confirms
that only the 12 Host source fields differ. Ordinary `paths` overrides were
rejected as the recommendation because Cargo warns about the Git-to-path
dependency graph change. The temporary `[patch]` strategy emits no such warning.
After parent integration, build the isolated TUI with
`cargo build --release --locked --config <temporary-patch-config>` using the
appropriate remote build lock/toolchain. This task verified resolution only,
not a TUI build or interactive/live-provider acceptance. All Cargo probes ran
remotely; the original TUI worktree remains untouched.

## Handoff

Implementation is a local topic-branch commit only; the following documentation
commit records its exact SHA and validation. Parent integration should take both
commits in order, retain 0.11.0 and the existing external pins, then perform its
own integrated TUI build/acceptance. No push or publication was performed.
