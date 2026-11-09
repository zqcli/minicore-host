# Add Linux Foundation CI

Status: implemented and checked locally; Host publication and the first hosted run remain pending. This is foundation coverage, not full SPEC v4.93 acceptance.

## Scope and commits

- Base: `d73b161e8d84d77a19aecd92125d11063192be05` on local `dev`.
- Workflow: `7cbbfc3df241b9b4b6a990eaeda668b5f7d37e06` (`ci: run the Linux foundation quality gate`), developed on `ci/linux-foundation` in a separate worktree.
- Owned files: `.github/workflows/ci.yml` and this task record. No source, manifest, lockfile, quality-gate script, or TUI changes.
- One Ubuntu 24.04 job runs on pushes to `dev`/`main`, pull requests, and manual dispatch. Rust is pinned to `1.90.0`, matching `rust-toolchain.toml`, with `rustfmt` and `clippy`. Update both pins together when changing the supported toolchain.
- The job calls the existing executable `./scripts/check.sh` directly. It does not duplicate or weaken the checks, skip failures, or substitute mock success. Only `contents: read` is granted; checkout credentials are not persisted. No cache, toolchain-setup, upload, or other third-party action is added.

## Action provenance

On 2026-11-09, the official [actions/checkout v4 ref](https://api.github.com/repos/actions/checkout/git/ref/tags/v4) resolved to `11d5960a326750d5838078e36cf38b85af677262`. The workflow uses that immutable commit, not the moving tag. The official [commit](https://github.com/actions/checkout/commit/11d5960a326750d5838078e36cf38b85af677262) and [action manifest](https://github.com/actions/checkout/blob/11d5960a326750d5838078e36cf38b85af677262/action.yml) were read to verify the pin and `persist-credentials` input.

## Local verification

On 2026-11-09, a Linux x86_64 worktree at the base above plus this workflow passed the full gate. Cargo used the published Runtime Git revision `18a6ac9b82d003330cd870275efca793cdcc9a96` from the existing manifests and lock, with no local `.cargo/config.toml` patch. The target directory started empty; the machine's Cargo registry/Git source cache and installed toolchain were reused. This is not a fresh hosted-runner result.

Exact gate invocation from the worktree root:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export RUSTUP_TOOLCHAIN=1.90.0
set -o pipefail
./scripts/check.sh 2>&1 | tee /workspace/shared/minicore-host-ci-quality-gate.log
```

Exit status: 0. `rustc --version` returned `rustc 1.90.0 (1159e78c4 2025-09-14)`; `cargo --version` returned `cargo 1.90.0 (840b83a10 2025-07-30)`. `rustup component list --toolchain 1.90.0 --installed` included `rustfmt` and `clippy`.

The unchanged script ran, in order:

1. `cargo fmt --all -- --check`: passed.
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
3. `cargo test --workspace --all-targets --locked`: 190 top-level unit/integration tests passed (60 core, 47 protocol, 27 provider, 56 storage). A storage subprocess probe is part of its parent test, not an extra distinct case.
4. `cargo test --workspace --doc --locked`: 2 doctests passed.
5. `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`: passed.
6. `git diff --check`: passed. `git diff --cached --check` also checked the new staged workflow/task files.

`python3` with PyYAML 6.0.3 parsed `.github/workflows/ci.yml` using `yaml.BaseLoader` (so the GitHub Actions `on` key remains a string). Assertions verified the three triggers, `dev`/`main` push filter, sole Linux job, read-only permission, timeout, Bash shell, exact official action pin, disabled credential persistence, and exact toolchain/component agreement with `rust-toolchain.toml`. They also verified the three-step workflow, the unchanged `./scripts/check.sh` invocation, every command in that script, and the absence of conditional or continue-on-error steps. `bash -n` checked both run-step bodies and `scripts/check.sh`; all passed. These are syntax and alignment checks, not a substitute for GitHub's workflow engine.

## Limits and next step

- `actionlint`, `yamllint`, and `shellcheck` were not installed (`command -v` found none); those linters were not run.
- The workflow has not run on GitHub because Host has not been published. The checkout action and installation on a clean Ubuntu hosted runner have not been executed locally. No remote push, workflow dispatch, branch-protection change, or other remote write was made.
- Host durable storage is Linux-only at this stage. This workflow does not validate Windows/macOS Host support, packaging, live external model providers, TUI integration, or the unfinished SPEC acceptance surface. Scripted model and loopback HTTP fixtures remain explicitly test fixtures.
- After review and the owner's explicit approval to publish Host, inspect the first hosted run at the exact pushed commit. Report actual results and any runner/network failure; do not treat this local pass as remote CI evidence.
