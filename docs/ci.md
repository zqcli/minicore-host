# Native CI and distribution packages

The [CI workflow](../.github/workflows/ci.yml) runs for branch pushes, `v*` tag
pushes, pull requests and manual dispatch. It uses the repository's pinned
Rust 1.90.0 toolchain and committed Cargo.lock throughout.

CI disables incremental compilation and debug symbols for dev/test profiles to
control disk and linker usage across all test targets on clean runners. It keeps
their normal optimization level, debug assertions and overflow checks, and does
not change the release profile or the repository's local build defaults.

## Required gates

The Linux quality job runs the complete `scripts/check.sh`: historical and
working-tree whitespace checks, the existing Python evidence checks, rustfmt,
strict Clippy, all workspace tests, documentation examples and strict rustdoc.
It checks out full Git history for the committed-whitespace check.

Each native platform also runs strict Clippy, all workspace targets and
documentation examples. A failed matrix member remains a failure and does not
cancel the other members. Packaging requires both the quality gate and the
complete native test matrix to pass.

The Windows job additionally runs five existing storage and Host process
contracts as a temporary ordinary account with its actual OS profile loaded.
These cover object/history save and reopen, explicit deletion, stable Host
identity and a real loopback HTTP model request persisted by Host. The helper
checks the account SID, rejects administrator membership, verifies each exact
test exists and requires one executed passing test per selection. The account
can read/execute the compiled programs but cannot modify the checkout; test
data belongs to its own profile. Account, processes, profile and temporary
permissions are cleaned up even after failure.

| Native runner | Release target | Archive |
| --- | --- | --- |
| `ubuntu-24.04` | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| `macos-15` | `aarch64-apple-darwin` | `.tar.gz` |
| `macos-15-intel` | `x86_64-apple-darwin` | `.tar.gz` |
| `windows-2025` | `x86_64-pc-windows-msvc` | `.zip` |

All four native jobs and all four packages pass at source
`f29d3c9c7b7d2f10ed6bcc72a131ca3d85cd37a6` in
[run 43068086506](https://github.com/zqcli/minicore-host/actions/runs/43068086506).
[Task 105](../tasks/105-multiplatform-ci/TASK.md) records exact test counts,
standard-user execution, final review and artifact hashes. Platform support
remains subject to the prerequisites below.

## Host and Runtime are one executable

Every package build invokes:

```sh
cargo build --package minicore-host --release --locked --target <target>
```

Cargo compiles the Runtime library through Host's existing dependencies and
links it into `minicore-host` (`minicore-host.exe` on Windows). Runtime is now
the internal `crates/host-runtime` package; all workspace packages use version
**0.10.1**. Its source was imported from the previously pinned Runtime commit
`3aa47629929906832dcb309afb7d242c1ae3e53f`, then refactored under SPEC 4.10.1.
Session owns the complete Turn and calls model/tool primitives directly.
CI builds that same workspace; no separate Runtime repository or process is needed.

## Packages

The packaging helper obtains Host's actual workspace version through Cargo
metadata and executes the copied native release binary's `--help` from the
staged package directory. This must exit
successfully, leave stdout empty and write usage to stderr, preserving stdout
for protocol frames.

Each archive has one `minicore-host-<version>-<target>` directory containing
the binary under `bin/`, [example.host.json](../example.host.json), executable
and component guides, the tool-approval guide and the project/specification
documents. Unix executables retain mode `0755` inside the tar archive. Links to
source and historical records not included in the package point to that build's
exact Git commit; links between bundled documents remain relative.

After creating the archive, the helper reads it back and requires exactly the
staged files, one Host executable, and the same executable SHA-256 as the binary
that passed the native smoke check. Unix archives must retain mode `0755` for
Host and contain no other executable files or links. The job prints both Host
and archive SHA-256 values for verification against the downloaded artifact.

Download the archive from a successful workflow run's **Artifacts** section and
extract it before starting Host. The example configuration deliberately uses an
invalid placeholder provider endpoint. Supply your actual provider, model and
capabilities, and provide credentials only through the named environment source.
The [executable guide](../crates/host-app/README.md) describes initialization,
stdio/WSS access, tool selection and controlled shutdown.

Tag pushes run the same gates and create workflow artifacts. This workflow does
not create tags, publish a GitHub Release or bypass validation for a tag.

## Platform prerequisites

Unix storage requires real file/directory synchronization, atomic same-directory
replacement and advisory file locks. The Windows adapter requires local NTFS and
checks actual volume, file identity and permissions. The current user's existing
OS Profile is its documented namespace boundary; data roots elsewhere require
write/sync access to every ancestor. See the [storage guide](../crates/host-storage/README.md)
for the precise ownership and durability assumptions.

The Bash tool uses `/bin/bash` on Linux/macOS and an installed native Git Bash on
Windows. Host binds Windows children to a Job Object and readies its completion
monitor before they run. Cleanup requires that exact Job's completion notification,
the joined monitor and process wrapper, and completed pipe handling; a zero Job
accounting count alone is not completion evidence. Runtime itself is part of the Host
executable; these operating-system and optional local-tool prerequisites remain
external. Native CI verifies the declared runners, not every OS or filesystem.

## Test-only history inspection and failure injection

Windows history locks intentionally reject unrelated file reads and writes while
the real journal owner remains active. Test fixtures that inspect bytes at an
established business boundary use an actual read-only file mapping on Windows;
this does not release the writer or manufacture data from in-memory records.
Failure and corruption fixtures operate on the journal's existing owned handle,
preserving its original accepted range, pending bytes and reconciliation path.

These cross-crate fixtures are available through storage's non-default
`test-support` feature, enabled only by development dependencies. Normal release
build commands do not enable it. A mapped read is an observation of actual bytes,
not an acknowledgement that those bytes have been synchronized successfully.

## Permissions and reproducibility

Actions use verified commit pins; workflow permissions are `contents: read` and
checkout does not persist credentials. No live-provider secret is required for
the local HTTP/SSE fixtures used in CI. Passing these fixtures does not assert
live-provider or TUI validation.
