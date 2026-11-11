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

## Native implementation checkpoint

The first CI source is remote `30a33abf64b20d298e0001822f4699df1e9362cf`, with
tree `44dee3ed5c85ec3bbc30151df3270fac19a366ac`, identical to local `2040f163`.
[Its native run](https://github.com/zqcli/minicore-host/actions/runs/43030627462)
passes both Linux gates. macOS ARM64 exposed five platform assumptions: FIFO
locking, two canonicalized temporary-path sync hooks, workspace root aliases
and Bash 3.2 variable-presence syntax. These have targeted fixes, preserving
real locking, fault injection and credential assertions. Windows was explicitly
blocked by the unfinished storage backend at that checkpoint.

The [standard-user Windows probe](https://github.com/zqcli/minicore-host/actions/runs/43031057105)
also passes on source `87b87201e2ba1f23cef233d71a59294dbcad539f` and confirms
`IsAdministrator=false`. Directory flushes work inside that user's profile;
opening `C:\Users` or the volume root for write is denied. The implementation
therefore uses the actual OS Known Folder Profile as an explicit existing
namespace boundary. It pins that directory before any Host mkdir, flushes the
profile and every descendant ancestor on acquisition (including failed earlier
creation attempts), and retains read-only no-delete handles above it. Roots
outside that profile still require every ancestor's real write/sync permission.
There is no PermissionDenied or filesystem-sync no-op fallback.

Windows storage now checks actual NTFS volume identity, owner/DACL permissions,
file IDs, hardlinks and reparse points. Directory share modes provide the root
lease and stable owned ancestry; journals retain actual OS file locks. Explicit
deletion drops the now-empty target guard before removing its parent entry.
Native confirmation of the full backend remains pending at this checkpoint.

Review found a concrete Windows cleanup defect in process-wrap 8.2.2 and released
10.0.1: their completion-port wait can accept a notification while descendants
remain alive. The dependency is now fixed to official source
[`2f4bc393d8bd03b4b1c81fee2e13ce82dd42906e`](https://github.com/watchexec/process-wrap/commit/2f4bc393d8bd03b4b1c81fee2e13ce82dd42906e),
version 10.0.1, which checks actual active-process accounting. A native Windows
regression keeps a real descendant alive after its leader exits and requires
wait to remain pending until that descendant is explicitly released. Private
adapter API changes pass the Linux process/app regressions and Windows cross
Clippy; Runtime's pin and interfaces are unchanged.

The second CI source is remote `271a18b3de034f9560fc1e4ece3cc30f3a476c16`,
tree `8e2bfc01dcf4b4ced81ec6f8e79491d6a19879b3`, identical to local `f3d3330f`.
[Its run](https://github.com/zqcli/minicore-host/actions/runs/43033621867) passes
both Linux gates. Windows compiles the production crates, then exposes
Unix-only pipe assumptions in transport tests. Shared test fixtures now use
actual Unix pipes or Windows named byte pipes, retaining real OS endpoint
lifetimes and backpressure. Tests previously excluded only for Unix pipes now
execute on Windows as well. The new ordinary-user CI step runs five existing
storage/app contracts and rejects missing, skipped or zero-test results.

Final review also replaces per-boundary Windows history hashing with a native
history lease that denies write sharing, including existing writable mappings.
The original byte locks, file-identity checks and uncertain-write prefix hashes
remain. Real mapping and child-process tests cover those lease boundaries;
the ordinary metadata path retains its original sharing. Windows atomic local
file replacements now preserve the source owner and effective DACL, including
protected/unprotected inheritance, before writing body bytes. Unsupported owner
changes or mismatched inherited ACLs are explicit failures.

Local Linux `scripts/check.sh` passes at the second checkpoint, including
965 unique Rust cases, 4 doctests and 28 Python checks. The native Linux release
build and packaging helper also pass: the archive contains one executable with
mode 0755, example configuration and documentation; `--help` succeeds on stderr
with empty stdout. `ldd` reports only system libraries, no Runtime shared object.
This local package is validation evidence, not a published final release.

Full final gates, independent final review, final source identities, native CI
results and four-platform artifact verification are pending. A configured
matrix, cross-check or standalone probe does not establish platform support.
