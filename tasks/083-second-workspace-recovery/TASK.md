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

## Fresh recovered checkpoint and integration

The recovered initial owner/Session slices and their original review corrections
are now integrated. The four-file close bridge is present; Session close/terminal
review approves source `87fa305`, and initial Host/Agent lifecycle review approves
`7781a1d`. The query pipe teardown was independently reimplemented with controlled
red probes; its three-case fixture passes 200 producer and 20 integrator repeats.
Cross-worktree Cargo target reuse produced stale-artifact false failures during
review; fresh worktree-exclusive targets passed without source changes. Targets
are now separate for every worktree.

Source `101abcb25a416a81f8350ca1acdbe6078b0de0bc` is published as
`9f75d0e770b39b99614530fa126fd90952f95fe7`, identical tree
`16cb1a24ea981d1fb4bd43c341ffede2567634c8`. Three verified recovery backups
preserve 353 published commits and unchanged main. The cumulative publication
map now has 70 entries, including the explicitly labelled recovered 57-entry
prefix. Its original recovered record is retained in
`docs/recovery-publication-8c5fe49.json`. The fixed [review handoff](../../docs/recovery-checkpoint-review.md)
links exact source/remote identities and separates pending review.

Fresh commands at source 101abcb all exited 0:

```sh
cargo test -p minicore-host --test credential_environment --test task084_app_log_fixture --locked --offline
CARGO_NET_OFFLINE=true ./scripts/check.sh
cargo test --workspace --all-targets --locked --offline -- --list
```

The gate has 899 unique Rust cases, 4 doctests and 20 Python checks, with strict
lint/docs, fmt and committed whitespace. The [manifest](../../docs/recovery-checkpoint-validation.json)
records new log hashes. It supersedes neither older evidence nor future checks.
Exact-head hosted CI 42739672016 was still running at the last observation.

The preceding full local 985ff19 gate and remote ee9 CI failed because new owner
host.started diagnostics exposed an unqualified process-readiness fixture lookup.
The actual credential assertions had not run. Source 101abcb qualifies component
and event and passes the real credential and provenance regressions; production
credentials, environment filtering and timeouts were not changed. A separate
earlier hosted TLS deadline fixture race remains under deterministic correction.

Next source `eb1065df9aa948478b907bd06407b56665eea955` integrates
`7c7efd9` as `32a3820` (Session history/metadata facts) and `19e7bbb` as
`eb1065d` (Agent metadata/reconcile), after documentation source `e95b805` as
`ac181fc`. The focused diagnostic-storage, Agent metadata and callback-lock targets
pass, as do strict core/transport all-target Clippy, fmt and whitespace. Their
combined gate and independent new-scope review remain separate pending work.

## Verified next backup and subsequent work

Local d2d0c97c2d970cf05e294db71fd4bdd5c0dbdd06 maps to remote dev
cc2bf0fbd40f04920d611332ef0ff55adc724aec with identical tree
34266ef4a0566e3695fe3e3eaa0aad92ef8fb7d2. Exact-head CI42743579503 completed
successfully. The mapping now contains 80 entries and retains all previous 75;
main and all 363 remote commits are unchanged. Earlier logs/counts are not
retroactively assigned to this later source.

Subsequent integrated code f36761a adds actual control/Closed diagnostic facts;
34a0517 adds the fixed-source 52-method audit with its open saved-result P1.
Tasks 087/088/089/090 own saved-result correction, compaction diagnostics,
independent Session/model review and fresh performance evidence. Runtime 2f7aeb9
is independently reviewed locally, not published/pinned. Its isolated Host
consumer e4be354 remains separate, without a committed Cargo path override.

This documentation update validates 80 ordered mapping records, source/remote
head/tree, preserved prefix, JSON, links, SPEC/lock identity and whitespace.
It does not claim a new full source gate or closure of the saved-result P1.
