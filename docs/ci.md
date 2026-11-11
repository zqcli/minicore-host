# Native CI and distribution packages

The [CI workflow](../.github/workflows/ci.yml) runs for branch pushes, `v*` tag
pushes, pull requests and manual dispatch. It uses the repository's pinned
Rust 1.90.0 toolchain and committed Cargo.lock throughout.

## Required gates

The Linux quality job runs the complete `scripts/check.sh`: historical and
working-tree whitespace checks, the existing Python evidence checks, rustfmt,
strict Clippy, all workspace tests, documentation examples and strict rustdoc.
It checks out full Git history for the committed-whitespace check.

Each native platform also runs strict Clippy, all workspace targets and
documentation examples. A failed matrix member remains a failure and does not
cancel the other members. Packaging requires both the quality gate and the
complete native test matrix to pass.

| Native runner | Release target | Archive |
| --- | --- | --- |
| `ubuntu-24.04` | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| `macos-15` | `aarch64-apple-darwin` | `.tar.gz` |
| `macos-15-intel` | `x86_64-apple-darwin` | `.tar.gz` |
| `windows-2025` | `x86_64-pc-windows-msvc` | `.zip` |

The task record distinguishes configured targets from actual passing runs and
records platform prerequisites and outstanding blockers. See
[Task 105](../tasks/105-multiplatform-ci/TASK.md) before interpreting a matrix as
a supported release.

## Host and Runtime are one executable

Every package build invokes:

```sh
cargo build --package minicore-host --release --locked --target <target>
```

Cargo compiles the Runtime library through Host's existing dependencies and
links it into `minicore-host` (`minicore-host.exe` on Windows). Runtime remains
version **0.10.0**, fixed to commit
`3aa47629929906832dcb309afb7d242c1ae3e53f` in the workspace manifest and lockfile.
The workflow does not check out a different Runtime branch or build a separate
Runtime process. Host/Runtime request and ownership interfaces are unchanged.

## Packages

The packaging helper obtains Host's actual workspace version through Cargo
metadata and executes the native release binary's `--help`. This must exit
successfully, leave stdout empty and write usage to stderr, preserving stdout
for protocol frames.

Each archive has one `minicore-host-<version>-<target>` directory containing
the binary under `bin/`, [example.host.json](../example.host.json), executable
and component guides, the tool-approval guide and the project/specification
documents. Unix executables retain mode `0755` inside the tar archive. Links to
source and historical records not included in the package point to that build's
exact Git commit; links between bundled documents remain relative.

Download the archive from a successful workflow run's **Artifacts** section and
extract it before starting Host. The example configuration deliberately uses an
invalid placeholder provider endpoint. Supply your actual provider, model and
capabilities, and provide credentials only through the named environment source.
The [executable guide](../crates/host-app/README.md) describes initialization,
stdio/WSS access, tool selection and controlled shutdown.

Tag pushes run the same gates and create workflow artifacts. This workflow does
not create tags, publish a GitHub Release or bypass validation for a tag.

## Permissions and reproducibility

Actions use verified commit pins; workflow permissions are `contents: read` and
checkout does not persist credentials. No live-provider secret is required for
the local HTTP/SSE fixtures used in CI. Passing these fixtures does not assert
live-provider or TUI validation.
