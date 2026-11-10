# Task102: Release Host 0.9.10

## Request and source

The owner explicitly requested: "版本号改0.9.10，添加tag，然后推送远端".
The publication destination is `zqcli/minicore-host:dev`, with an annotated
`v0.9.10` tag. This includes the reviewed owner-path simplification from Task101.

- Starting local checkpoint: `0e82c682d55b9297559244291a84267430e06935`.
- Remote `dev` before publication: `6ea03b85a318374f3dcb94c38dbe11ac1878f110`.
- Simplification code: `0a40a5639069e39e3f304aac6fbff079adabd1d2`.
- Version checkpoint: `4301deb87353d4246e8e43be881e3a61a4a1c7ad`.
- The release tag includes the version checkpoint and this release record.
- No tag existed locally or remotely when this release was prepared.

## Changes and independent review

- All 12 workspace packages move from 0.1.0 to 0.9.10.
- All 34 internal path dependency requirements move to 0.9.10. The storage
  dependency on protocol keeps its exact `=0.9.10` requirement.
- Cargo.lock changes only the versions of those 12 workspace packages.
- Host discovery, provider User-Agent and application diagnostics already use
  `CARGO_PKG_VERSION`, so each reports the updated package version.
- Fixed diagnostic fixture values and historical evidence retain their original
  versions. No new CLI version flag or product behavior is introduced.

An independent read-only engineering review, requested with `gpt-6-astra` and
`max` reasoning, compared manifests and Cargo.lock structurally against the
starting checkpoint. After reversing the expected version fields, the parsed
files are identical to the baseline. This records the requested worker settings,
not independently attested effective runtime settings. No release blocker was
found.

SPEC v4.93 remains unchanged. Runtime stays at exactly 0.10.0, Git revision
`3aa47629929906832dcb309afb7d242c1ae3e53f`; every external dependency, source,
checksum and feature remains unchanged.

## Validation

The complete repository gate and test inventory both passed against the version
checkpoint with exit code 0:

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
```

The local environment uses Rust 1.90.0, cached dependencies in offline mode,
two build jobs, disabled incremental compilation and omitted development debug
symbols. Normal test assertions and development optimization are unchanged.

Cargo metadata and a structural lockfile comparison already confirm all 12
package versions, all 34 internal requirements and zero external dependency
changes.

| Check | Result |
| --- | --- |
| Whitespace and rustfmt | Passed |
| Clippy, all workspace targets, warnings denied | Passed |
| Unique Rust unit/integration tests | 955 across 130 targets |
| Doctests | 4 passed |
| Python regression checks | 28 passed |
| Rustdoc, warnings denied | Passed |
| SPEC SHA-256 | `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3` |
| Updated Cargo.lock SHA-256 | `9b9b63aac2274746f5b774add18f92f192956e4776211f535b0722249de3d2ef` |

The inventory counts test cases once; nested storage probe output is not added
again. Raw local logs are not bundled in the repository. Their fixed hashes are:

| Local log | Bytes | SHA-256 |
| --- | --- | --- |
| release-0.9.10-gate.log | 119403 | `ecf8a772548ddaeb8879b70dadee6defbda9dfe2ae042311504bf99e243b9f76` |
| release-0.9.10-inventory.log | 93966 | `ea39e1ba87fbbfb730262ca3be1fdb49b2fb223c46905e8422186a3775ed31c8` |

The final release-record commit changes documentation only; Rust source,
manifests, Cargo.lock and SPEC remain identical to the validated version
checkpoint. Whitespace checks also cover that final documentation update.

## Publication and verification

The current owner instruction supersedes the earlier automatic approval block.
The local annotated tag points to
`a8003501b8af23c6f5607571a3ca33cfcdd11f97`. A subsequent documentation-only
checkpoint records this publication outcome without moving that release tag.

The normal atomic push was attempted but failed with
`fatal: could not read Username for 'https://github.com': No such device or address`.
The owner completed the requested GitHub device login. The CLI process then
failed because network policy blocked `https://api.github.com:443`, before it
could persist login credentials. `gh auth status` still reported no logged-in
host. The environment's granular approval policy also rejected the supported
network escalation request (`sandbox_approval: false`). This is an environment
access limitation; the owner's publication authorization remains valid.

Remote `dev` remains `6ea03b85a318374f3dcb94c38dbe11ac1878f110`, and no remote
`v0.9.10` tag or new release CI run exists. The connected GitHub tools have no
tag-creation operation, so no partial alternate publication was attempted.

A complete Git bundle named `minicore-host-0.9.10.bundle` preserves local `dev`,
its full ancestry and the annotated tag. In an environment with GitHub push
credentials, the publication procedure is:

```sh
git clone --branch dev minicore-host-0.9.10.bundle minicore-host-0.9.10
cd minicore-host-0.9.10
git remote set-url origin https://github.com/zqcli/minicore-host.git
git push --atomic origin dev refs/tags/v0.9.10
git ls-remote origin refs/heads/dev refs/tags/v0.9.10 'refs/tags/v0.9.10^{}'
```

The branch update must preserve remote history, and the annotated tag's peeled
commit must match the published release checkpoint. Existing CI runs on `dev`
pushes; tag pushes alone do not trigger it. Hosted CI is a separate result from
the local gate and must be checked for the published commit.

Actual external-provider and consuming TUI acceptance remain outside this
version release. This task does not merge into `main` or publish crates.
