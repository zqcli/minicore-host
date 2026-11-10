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

## Publication through the connected GitHub API

The owner explicitly requested another submission through the already connected
GitHub account. Repository metadata confirms write permission. The supported
Git Data APIs successfully create trees and commits, independently of the CLI
login problem recorded below.

All six local checkpoints are reproduced in order with identical trees and
original commit messages. API-generated timestamps change commit SHAs; actual
author and committer are both `zqcli <276883664+zqcli@users.noreply.github.com>`.
There are no co-author trailers. The exact mappings, trees, parents and returned
identities are recorded in [publication.json](publication.json).

- Simplification: local `0a40a5639069e39e3f304aac6fbff079adabd1d2` maps to
  `4ba0afa0ad1d2022195be1a11a08cbacb19976ff`.
- Version bump: local `4301deb87353d4246e8e43be881e3a61a4a1c7ad` maps to
  `356c1c4c1c1c01bada0236041d22326551ce7431`.
- Original release checkpoint: local `a8003501b8af23c6f5607571a3ca33cfcdd11f97`
  maps to `ca8580653d0f069b60ba6bdb82460a35972b6ed8`.
- Complete local source head `674ec56ca125eea7143ef5630e066f57372001ab` maps to
  `58e1ee65e7b37f48acd0d57455aab7cd6fb9b2dc`, tree
  `01ff0dcf283dd5072dbcfd3bba9966a579fbf054`.

The final publication-record commit updates only README, PROJECT, Task101 and
Task102 records. Production code, tests, scripts, manifests, Cargo.lock and SPEC
remain identical to the validated 0.9.10 checkpoint. The `dev` update uses the
observed old head `6ea03b85a318374f3dcb94c38dbe11ac1878f110` as its expected SHA
and `force: false`, retaining all existing remote history. Hosted CI must be
checked for the actual final `dev` head; the local gate is a separate result.

### Tag status

Remote creation of `v0.9.10` remains pending. Its intended published target is
`ca8580653d0f069b60ba6bdb82460a35972b6ed8`, the exact tree of the original local
release checkpoint. The current connected tools expose branch/commit writes but
no tag-object or tag-ref creation operation. Their `create_branch` documentation
does not promise support for a `refs/tags/*` destination. A read-only check of
the cloud browser found GitHub signed out, so it supplies no authenticated tag
creation path either. These limits do not block the authorized source submission.

The original local source history and tag are retained as an archive. They are
not the current remote commit chain; the old bundle's direct `dev` push procedure
is superseded by this API publication mapping. The local publication checkout
and any refreshed bundle should track the actual published commits before a
later tag push.

### Earlier CLI attempt

The ordinary atomic push failed because Git credentials were unavailable. The
owner completed device login, but the CLI process was interrupted by the network
policy blocking `https://api.github.com:443` before credentials were persisted.
The environment also rejected the supported network escalation request. These
were limitations of that CLI route; the connected GitHub write APIs remain
available and are used for the source publication above.

Actual external-provider and consuming TUI acceptance remain outside this
version release. This task does not merge into `main` or publish crates.
