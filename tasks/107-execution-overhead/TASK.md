# Task 107: reduce execution overhead

## Scope and authorization

On 2026-11-13 (UTC), the owner authorized improvements 1–4 from the
Runtime/Host review and a paired performance recheck. The source baseline is
dev `5911d56a64bb2302e075b11cd44d667643999f07`, Host 0.10.1 and SPEC 4.10.1.
The existing specification, version, dependency pins and provider wire decoder
remain the baseline. This task does not implement review item 5.

## Changes and invariants

1. Make tool-output capabilities cheap to clone. Validate the original
   invocation when issuing them, then retain original Turn/Load/call identity,
   weak owner lifetime, revocation and actual worker cleanup confirmation.
2. Keep one model-output text buffer and an explicit complete-response fact;
   preserve partial results, history pairing and UTF-8 range reads.
3. Share frozen tool declarations and preserve them when binding implementations.
   Reuse validated execution configuration and borrow read-only policy inputs.
4. Estimate an automatic compaction request once, avoid repeated historical JSON
   materialization, and replace repeated within-exchange result scans with a
   small local set. Keep exact chunk accounting and all actual history content.

Accepted work, model partials, storage confirmation, tool cleanup and observation
remain separate facts. No new cache manager, execution quota, compatibility
layer, silent fallback, automatic tool replay or provider protocol change.

## Work plan

- Preserve an untouched source worktree and release baseline before measurement.
- Implement output, resource binding and compaction in disjoint worker scopes.
- Review the integrated change, run targeted contracts and the repository gate.
- Build both variants with pinned Rust 1.90.0 and unchanged external dependencies.
- In a coordinated no-build window, run balanced AB/BA paired measurements with
  identical content, output, provenance and outcome checks. Retain all samples.
- Publish the verified change to dev without rewriting history; follow native
  CI and release-package outcomes. Record exact source identities and limits.

## Ownership

Three workers are requested as `gpt-6-astra` with `max` reasoning: core output
capabilities/buffers, tool/resource binding, and compaction. This describes the
requested settings, not independently attested effective model configuration.
Root owns integration, shared request helpers, measurement tooling, project/task
records, validation and publication. Contributors do not make independent commits.

## Progress

- Confirmed remote and local dev match the baseline and the working tree is clean.
- Created a detached baseline worktree and local `perf/host-execution-107` branch.
- Read AGENTS and SPEC Rules; implementation keeps their existing behavior.
- Installed Rust 1.90.0, rustfmt and Clippy in this fresh workspace; the installer
  was checked against its official SHA-256. Rebuilt the unchanged baseline in
  release mode with four Cargo jobs and incremental compilation disabled.
- Implemented all four items, retaining SPEC 4.10.1, Host 0.10.1, dependency pins
  and provider decoding. Updated the affected internal crate documentation.
- Tool-output and cleanup integration targets pass 25 cases. The additional
  actual-Host measurement scenarios pass baseline correctness smoke checks;
  these concurrent-build smoke timings are discarded, not performance evidence.
- Independent worker review found no blocking issue in output identity,
  weak-owner lifetime, lease revocation and complete/partial response handling.
- Compaction passes 25 unit/engine/local-HTTP cases and two new prompt cases;
  the output partial-query regression also passes. Independent review checked
  JSON escaping, chunk carry-over, full result identity and manual semantics.
- The first workspace Clippy run found one old `wrap(tool)` call in the shared
  transport observation fixture. It now uses the same frozen `map_tools` binding
  as production. A second pass found one obsolete `ToolSet` test import, removed
  without changing production code. The final full gate passes.
- The additional performance runner compares full normalized request hashes,
  including instructions and request options. Paired baseline smoke checks
  verified that only actual message IDs, temporary workspace and checked command
  PID are normalized. Timings from these checks are not retained as evidence.

## Validation and performance

The exact measured production checkpoint is
`946b3c0d35d20ff7468211df18b600a21a576232`. The subsequent unused test-import
correction does not change any production source, manifest or dependency.
Both release variants and the same adapted Task104 compaction probe built
successfully. The full local gate passes 1,117 unique Rust cases, 6 doctests and
28 Python checks, formatting, strict Clippy/rustdoc and whitespace. The count
comes from `cargo test --workspace --all-targets --locked -- --list`, not nested
storage probe output. See [validation.json](validation.json).

Four balanced AB/BA paired rounds have completed with exact workload checks;
the detailed report and all raw aggregate samples are being finalized. Native
CI will be verified on publication. Earlier Task106 results remain historical
evidence and are not substituted for these new measurements.
