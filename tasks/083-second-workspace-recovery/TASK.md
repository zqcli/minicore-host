# Restore the second replaced workspace

## Verified boundary

The new environment had no previous Host checkout, worktree, Rust toolchain,
Cargo cache or raw evidence. A full clone restores remote dev
`8c5fe493f9b9e78e15ef831c532bc267d0c7fb53`, 340 commits, and unchanged main
`b341b163e14543bfbe7f8d392ea243b4a123cdf7`. `git fsck --full` passed.
SPEC and Cargo.lock retain their recorded SHA-256 values. Official Rust 1.90.0,
rustfmt and Clippy are installed; `cargo fetch --locked --target
x86_64-unknown-linux-gnu` and filtered offline metadata select only Runtime
`69faf9196991524b5549499d6208a1eaf567bf56`, version 0.10.0.

The last formerly verified local dev was
`627ce84260d51e565d974a8f8299517aeb35d050`. Its preceding unpublished batch
was `1ce6910`, `f7c45af`, `5fc6550`, `ad289ec`, `7cae78e`; later `7b7b154`
and `627ce842` added Span-drop correction and close diagnostics. None of these
local Git objects was recovered from remote. No tree/commit/ref write for that
next batch had started. The old commit names are evidence of previous work,
not available source identities or a publication claim.

## Recovery approach and current sources

- Source `8310c2826eb918182c8eef83878faa9125af6e1f`, integrated as
  `67cbd836accaa7dab90e945c64752461029080a4`, restores the exact private
  task071 fixture handling: emit the returned Record outside the direct call.
- Source `f4b105c11eb54c2a9df8a0671f7c0dbf9a9b7861`, integrated as
  `d6d5d34524d82a38b467e1df3ef28fa325f74beb`, replays retained complete
  terminal/save edits and the 172-line actual HTTP/history regression. Original
  per-file diff statistics match, but unavailable old Git objects prevent a
  full old-tree equivalence claim. New checks are required.
- The original four-file Host/Agent close bridge patch was recovered with exact
  SHA-256 `e439b94e2f45ba4cd9c8702d62cc371fc9b05fa6f50d83d5effd0f8b3b008c14`.
  Session implementation and its final private-fixture adaptation are being
  rebuilt separately; patch recovery is not final-composition validation.
- Host/Agent lifecycle source and its two known review corrections are being
  reconstructed from retained implementation steps. Original Span-drop and
  persistent-mode probes are recoverable; old raw outputs are not presumed.
- task081's final source was not retained in its resumed context. Its narrow
  pipe-fixture fix is a new implementation based on public task054 code and
  fresh controlled reproduction, not byte-identical recovery.

The publication map has all 57 known source/remote correspondences. Its first 37
entries came from a byte-identical public JSON copy; 20 later entries were
restored and checked against actual published Git objects. The resulting file
is labelled recovered evidence; missing timestamps and original map bytes are
not invented.

## Validation and remaining work

Remote CI 42723000914 fails at task071's must-use Record under strict Clippy;
its later Rust tests did not run. Earlier b32 CI 42722187889 failed at task070's
strict server join with BrokenPipe. These are separate fixture issues.

New source checks must identify their own commit and logs. Older 858/4 full-gate,
owner-slice and independent-review results are historical, not new evidence.
Restore the remaining owner source, rerun deterministic tests and independent
reviews, then run a new combined gate and performance measurements. Backup every
five commits on dev while preserving all remote history; do not merge main.
