# Single-file apply_patch implementation and verification

This subtask follows SPEC v4.10.2, committed before implementation as
`32ea194ff2269aa8fb2bebff79355b600d404b45`, SHA-256
`55476e6d2c6c9ec5b857f2b9235834a162b3ad85ce56a8e2a295135f7dc370c7`.
The requested worker configuration was `gpt-6-astra` with `max` reasoning;
effective runtime settings were not independently verified.

Implementation commit: `3aa8e5720bb6e970ab7b1bfc9c083adb28115638`
(`feat(local-tools): add exact single-file patch execution`). The coordinator
cherry-picked it to the integration branch as
`c85b2e2` before final integration verification. No remote was pushed by this
worker, and no co-author trailer was added.

The approved contract is required `{path, patch}` for one existing UTF-8 text
file, accepting exact unified diffs or one Codex Update File envelope. Add,
Delete, Move, multifile, ambiguous and malformed patches are explicitly rejected.
The parser and newline assembly reuse minicore-agent reference
`f36a87366d8e2fe0a1c8216463887902e471c739` under
`src/tools/apply_patch/{apply,codex,unified}.rs`. Host adaptation removes the old
512 KiB and matching-work quotas and checks the actual tool cancellation/deadline
through parsing, matching and assembly. LF/CRLF, whitespace, source positions,
quoted UTF-8 header paths and side-specific no-final-newline markers retain their
mature behavior. This is capability reuse, not legacy configuration or history
compatibility.

`Mutation::Patch` reuses the existing actual Workspace mutation guard, source
capture, permission preservation, same-directory atomic replacement, conflict
check, directory sync and authoritative after-capture. The existing owned
LocalTools scope keeps accepted work alive if a caller drops its wait. Tool JSON
does not carry owner identity, and no Workspace public API, new filesystem owner,
external patch subprocess, Runtime protocol or TUI change was introduced. Native
cancellation errors and internal sink failures remain distinct from business
conflicts; committed effects are neither rolled back nor retried.

Verification ran with Rust/Cargo 1.90.0 on Linux. Commands used
`PATH=/home/agent/.cargo/bin:$PATH`:

- `cargo check -p minicore-host-local-tools --all-targets --locked`: passed.
- `cargo test -j 2 -p minicore-host-local-tools --test patch --locked`: all 16 new
  cases passed.
- `cargo test -j 2 -p minicore-host-local-tools --all-targets --locked`: all 48
  native Linux cases passed, including existing filesystem, process, Runtime and
  scripted loopback HTTP composition tests.
- `cargo clippy -j 2 -p minicore-host-local-tools --all-targets --locked -- -D warnings`:
  passed.
- `cargo test -j 2 -p minicore-host-local-tools --doc --locked`: passed, zero
  doctests in this crate.
- `RUSTDOCFLAGS='-D warnings' cargo doc -j 2 -p minicore-host-local-tools --no-deps --locked`:
  passed.
- `cargo fmt --all -- --check` and `git diff --cached --check`: passed.
- `cargo test -j 2 -p minicore-host-local-tools --all-targets --locked -- --list`:
  48 unique cases (4 unit, 12 files, 1 HTTP fixture, 16 patch, 11 process,
  4 Runtime boundary). The Windows-only permission suite lists zero on Linux.

New coverage includes source/patch/result above the former byte quota, matching
work above the former work quota, failed later hunks without partial publication,
exact edit evidence, shared Workspace serialization across Turn bindings,
cancel/deadline while awaiting the mutation guard, dropped caller waits and a
real committed patch followed by a deliberately failing record sink. The
existing native Windows DACL test now also patches protected and unprotected
files; it was not executed here and requires native Windows CI. Live providers
and the TUI were not run or claimed. Full workspace verification, version/lockfile
updates, integration publication and release remain with the coordinator.

Independent read-only review also covered the coordinator's explicit provider API
configuration/discovery, exhaustive resource construction, patch policy, example
packaging, Responses ordinary/manual-compaction/reload fixtures, real Session
patch record integration and binary read-only denial case. No correctness or
contract issue was found. A malformed patch string in the binary denial fixture
was reported and corrected by the coordinator to a valid Codex Update envelope.
That review did not substitute for the pending integrated runtime tests.
