# Task 105: native multi-platform CI

## Scope and source

The owner requested Host CI like minicore-agent, with Runtime compiled into the
Host executable, and explicitly deferred Host/Runtime interaction changes.
Baseline: `dev` commit `6ea85fda737635e26b9d7cad17c3df1e54911bd9`.
Reference: minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739`.
Implementation topic: `ci/multiplatform-host`.

Version remains 0.9.20; Runtime remains 0.10.0 at
`3aa47629929906832dcb309afb7d242c1ae3e53f`. SPEC v4.93 retains SHA-256
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
No changes to Runtime interfaces, Turn/Step ownership, compaction responsibilities
or TUI are in scope. Existing historical evidence is preserved.

## Work in progress

- Preserve the complete Linux quality gate and add native Linux x64, macOS ARM64,
  macOS x64 and Windows x64 Clippy/test/doc-test jobs.
- Gate four native release builds and archive uploads on all required checks.
- Build the existing Host Cargo package, including its pinned Runtime dependency.
- Package a real binary, initial configuration template and usable documentation;
  execute the native binary before publishing an artifact.
- Resolve platform compilation, filesystem and process-supervision prerequisites
  without substituting no-op saves, skipped platform tests or fake cleanup.

Three workers were requested with `gpt-6-astra` and `max` reasoning for disjoint
CI/packaging, storage and other platform-adapter scopes. These are requested
settings, not an independent runtime attestation. The root owns shared manifests,
the lockfile, top-level records, integration and publication.

## Initial validation and investigation

- Rust 1.90.0 `cargo check --workspace --all-targets`: passed on local Linux.
- `git diff --check`, workflow YAML parsing, Python helper compilation and
  official actionlint 1.7.12 validation: passed for the initial workflow.
- Cargo.lock adds only the 11 Windows dependencies required by process-wrap's
  existing pinned 8.2.2 Job Object feature; existing versions and Runtime pin
  remain fixed at this checkpoint.
- The original storage explicitly rejected non-Linux platforms. macOS reuses its
  Unix implementation; Windows needs actual native locking/path/sync semantics.
- An isolated standalone Windows API probe was published on
  `ci/windows-storage-probe`, commit
  `4ee7a94f2fe7c95cab65febe771c13fe3169b313`, with identical tree to local
  `e143ee4`. [Native run](https://github.com/zqcli/minicore-host/actions/runs/43029084962)
  passed: read/write directory handles completed real synchronization at all
  mutation phases; read-only and attributes-only handles failed with OS error 5.
  This runner used its administrator account. A separate non-administrator probe
  is pending. API success alone is not a power-loss durability test.
- The Windows adapter additionally pins winapi-util 0.1.11, windows-permissions
  0.2.4 and winsafe 0.0.29. Cargo adds six packages for these adapters without
  changing existing resolved versions. Windows requires real NTFS directory
  flushes, file identity and access checks; implementation is still in progress.
- Native process, workspace, diagnostic and executable integration targets pass
  on Linux after the portability changes. The Windows local-tools library also
  cross-checks successfully with `cargo check --locked --target
  x86_64-pc-windows-msvc -p minicore-host-local-tools --lib`; cross-checking is not
  native process execution.

Full gates, independent final review, final source identities, native CI results
and artifact verification are pending. This record does not claim that a
configured matrix or standalone probe establishes platform support.
