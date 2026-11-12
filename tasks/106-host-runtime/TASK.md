# Task 106: integrate the Host execution layer

## Scope and authorization

On 2026-11-12 the owner approved moving the Runtime implementation into the
Host workspace, replacing the standalone AgentLoop lifecycle with a single
Session-owned Turn driver, and publishing Host 0.10.1 with tag `v0.10.1` after
verification. The design baseline must first become SPEC 4.10.1. The owner
explicitly permits breaking internal APIs and deleting unused interfaces.

Source Host: `685806af4761eaaa75bae11cd70e5d7a7a0177b0` on `dev`.
Imported Runtime source: `3aa47629929906832dcb309afb7d242c1ae3e53f`, the exact
0.10.0 revision used by that Host, not the Runtime repository's different dev.
Implementation branch: `refactor/host-runtime-0.10.1`.
The unchanged TUI design baseline remains version 0.9.9 at
`3beeb1d8a525e58ce6352e05061f242c5580e248`; this task does not modify its repository.

## Design and invariants

- `host-core` Session owns input claims, one public Turn's execution sequence,
  configuration binding, original history, save ranges and terminal decisions.
- `host-runtime` is an internal workspace package providing shared model-call
  and tool execution mechanisms and the native values used by other modules.
- `host-provider` retains provider-specific HTTP and protocol adaptation;
  concrete tool modules retain actual side effects and resource cleanup.
- Normal and compaction model calls share the same proven model driver; summary
  calls have no tools or public Turn and keep an explicit single-attempt policy.
- Preserve input ordering, immutable Turn configuration, accepted-versus-saved
  facts, bounded safe model retries, actual partial output and real tool cleanup.
- No second runner lifecycle, automatic tool replay, durable queue recovery,
  quotas, speculative compatibility wrapper or new general management platform.
- Avoid repeated full-history clones, I/O under owner locks and extra execution
  tasks. Measure selected repeatable execution/context paths without claiming
  live-provider latency improvements from synthetic measurements.

## Work plan and ownership

1. Update and independently review SPEC/AGENTS before implementation.
2. Import the pinned Runtime sources and applicable tests/licenses, establish
   the workspace dependency and Host 0.10.1 versions.
3. Implement shared execution primitives and the Session-owned Turn driver;
   integrate compaction and remove unused standalone APIs and their obsolete tests.
4. Run focused regression checks, the full local project gate and paired release
   measurements; independently review the integrated source.
5. Preserve the owner's dev-only remote branch layout by fast-forwarding dev
   after local verification. Verify its four native test jobs, quality job and
   four release packages before creating the requested tag. Preserve remote
   history and use the original dev head as the update lease.

Three engineering workers were requested as `gpt-6-astra` with `max` reasoning.
This records the requested settings, not independently verified effective model
configuration. Their initial disjoint scopes are SPEC/AGENTS, host-runtime,
and host-core Session execution. Root owns integration, other crates, shared
manifests, release records and publication. Shared interfaces are coordinated
before edits.

## Progress

- Confirmed remote dev still equals the source above; the working tree was clean.
- Read current engineering instructions and the full SPEC Rules.
- SPEC 4.10.1 was committed first as `b490ed9`; SHA-256
  `b1426061eb63478f392e8f5638792df1c7d8a1bad08d80c80f3203288816182b`.
- Imported the exact pinned Runtime into `crates/host-runtime`, changed the
  dependency to a workspace path, renamed Rust imports and advanced all 13
  workspace packages to 0.10.1. The standalone API replacement followed this import.
- Provisioned the pinned Rust 1.90.0 toolchain with rustfmt and Clippy. The exact
  original Host baseline builds successfully in release mode in a separate
  worktree/target directory.
- Imported source/version/dependency changes were committed as `d482060`.
  Every external locked dependency retains its previous version, source and
  checksum; all 13 internal packages are local 0.10.1 packages.
- Replaced the standalone loop with direct Session model/tool calls. Inputs
  are authored once at binding, ordinary requests share the accepted native
  history buffer, and original save ranges survive a failed save. Removed loop
  controls, input/history callbacks, cumulative reports, mutable loop settings,
  unused history-size accounting and duplicate compaction response assembly.
- Ordinary and summary calls now share `ModelDriver`. Summary calls use one
  attempt. Focused compaction tests pass, including usage/cancellation and actual
  loopback HTTP/SSE. Existing local-tool and agent-tool tests pass after moving
  mechanism tests to direct tool batches and retaining real Host owner tests.
- Provider HTTP/runtime and diagnostic tests pass after migration to direct
  model calls. Their explicit two-request tool exchanges remain actual HTTP
  and tool execution, not a compatibility loop.
- Independent source review identified and corrected separate compaction
  deadlines, upstream error classification, genuine usage-only partials, and
  old-history identification by its actual native Turn correlation rather
  than a history slice boundary. The corresponding regression checks passed in
  the complete workspace gate.
- Prepared reproducible release probes for same-Turn continuation and long
  history, plus compaction calculation. Baseline correctness passes; timings
  collected while builds run are explicitly excluded from performance evidence.
- Fixed remaining integration assertions to reflect input authoring at binding,
  fixed native Turn identity and continuous request indexes. Cold-read content,
  complete old-history prefixes, actual message provenance and original journal
  bytes remain checked. Corrected the provider README's compiling example to
  use the internal crate name.
- Final deadline review removed the placeholder outer deadline. `ModelCallParams`
  carries the actual optional enclosing deadline; the driver constructs each
  provider's mandatory effective deadline once per logical request and shares it
  across retries. Compaction uses its real remaining budget and original absolute
  deadline for every chunk. Existing legal timeout/retry ranges are unchanged.
  Two paused-clock regressions verify absent outer deadlines and shared retry
  budgets. Runtime now passes 137 cases and two doctests.
- Final production checkpoint is `1009c719ced338ebb05c5bacccc8c9403f882ec2`,
  tree `5e1433f2c9fb874da7feb4fdec78388de6c8c514`. Its source-identical GitHub
  commit is `982593255fcf9b31ef1cd30a1b668420120e2ac6`. Independent review found
  no blocking issue in the final core/compaction deadline changes or in retained
  native process cleanup assertions. This is source review, not non-Linux testing.
- All 225 external locked packages retain their versions, sources and checksums.
  All 13 workspace packages are local 0.10.1 packages. The final Linux release
  executable builds successfully; the complete local gate and all nine native CI
  jobs pass. All four release packages were downloaded and independently verified.

## Validation and publication

The complete local gate passes on `6b2f217886e3952aafa1c4b297476101940a68c1`:

| Check | Command and result |
| --- | --- |
| Full gate | `CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 ./scripts/check.sh`, exit 0 |
| Rust cases | `cargo test --workspace --all-targets --locked -- --list`: 1,110 unique cases across 138 targets |
| Documentation examples | 6 passing doctests |
| Python checks | 28 passing checks |
| Static/documentation checks | rustfmt, strict workspace/all-targets Clippy, strict rustdoc and whitespace pass |
| Linux release | `cargo build --package minicore-host --release --locked --target x86_64-unknown-linux-gnu`, exit 0 |
| Linux package | `python scripts/package-release.py x86_64-unknown-linux-gnu`, exit 0; actual staged Host runs, exact 19-file archive and executable SHA/mode verified |

Counts identify `(target executable, case name)` from `--list`; nested storage
subprocess probes are not added again. The only change between the measured
production checkpoint `1009c719` and the final tested source is a transport
fixture. It now waits through the precisely identified original-owner return
window before resuming a failed save, keeping its multi-threaded executor,
original operation/candidate and all final result/HTTP-count assertions.
It does not retry a successfully accepted operation or change production code.

The initial integration gate found obsolete native Loop/index/input assertions
and one README doctest import; these were corrected before the passing final
gate. The final source contains the explicit optional deadline correction and
its two regressions. Exact commands, source identities and log digests are in
[validation.json](validation.json).

### Performance and source size

Both binaries and compaction probes were built from fixed commits using Rust
1.90.0. Measurements ran in coordinated windows without project compilation;
this does not establish machine isolation. [The report](performance/README.md)
and [comparison](performance/comparison.json) retain all initial three paired
rounds and all four balanced confirmation rounds, including every sample and
the workload-equality checks. No payload, transcript or opaque compressed log
is published with these aggregates.

The confirmation medians change from 1.146 to 0.982 ms for ordinary same-Turn
continuation and 2.952 to 2.436 ms after the large history setup. Compaction total
medians change from 4.885 to 5.175 ms (256 groups) and 18.976 to 20.357 ms (1,024
groups): a slowdown signal of 5.9% and 7.3%, respectively. The initial run has
different directions and order effects; its apparent 48.6% improvement on the
large compaction fixture is not a reliable benefit claim.

The inspected compaction history-processing path and Runtime value/request
serialization add no new size-dependent traversal or deep copy. The shared
driver performs one call on a fixed short response in this fixture. Request
retention lasts through stream completion rather than possibly ending in the
fixture's `start`; code generation, layout and allocator/runtime effects were
not profiled. No reliable causal attribution or supported hot-path fix was
established. Keep the shared safety implementation and report this limitation;
do not weaken validation or add special branches to improve an unexplained
measurement. This task claims neither universal speedup nor proven no-regression.

[Source-size comparison](code-size.json) records the imported Runtime and Host
together against the new workspace. The `src` directories have 2,410 fewer
physical Rust lines and eight fewer files. Physical lines include inline tests,
comments and blank lines; this is not a production-only LOC or performance claim.

### Publication

The original remote dev `685806af4761eaaa75bae11cd70e5d7a7a0177b0` was
fast-forwarded with an explicit lease and `force=false` to
`1e198a8af2643c64ac4497ac006834adfe65f50d`. Fetch verified both the exact tree
`63328a524b8a2d73bfbac7c044efada130367c21` and the original ancestry. No remote
topic branch was created. [publication.json](publication.json) maps the local
preparation commits to the source-identical connector commits, including the
SPEC-first checkpoint.

[CI run 43242142126](https://github.com/zqcli/minicore-host/actions/runs/43242142126)
completed successfully at `2026-11-12T15:22:13Z` on its first attempt, with all
nine jobs passing: quality, four native test jobs and four gated package jobs.
[ci-source.json](ci-source.json) records source identity, job outcomes, unique
case counts and artifact hashes.

| Native platform | Unique Rust cases | Test targets | Doctests |
| --- | ---: | ---: | ---: |
| Linux x64 | 1,110 | 138 | 6 |
| macOS ARM64 | 1,103 | 138 | 6 |
| macOS x64 | 1,103 | 138 | 6 |
| Windows x64 | 1,091 | 138 | 6 |

The Windows job also passes five existing checks under an ordinary user;
these re-executions are not additional unique cases. All four artifacts were
downloaded and verified against GitHub metadata and native packaging logs.
Each archive contains exactly 19 files and one Host executable; archive and
executable hashes match, and Unix executable modes are correct.

The final publication adds only `PROJECT.md` and Task 106 evidence records to
the verified code. Production source, tests, dependency pins and CI workflows
remain byte-identical to the source above. Documentation-only checks cover
whitespace, structured evidence and relative links; the recorded native run
does not attest completion of a later documentation or tag-triggered workflow.

The owner has authorized `v0.10.1` on the final dev commit including these
records. Native verification is complete. The available connector only exposes
branch ref updates, and terminal Git lacks push authentication. The remaining
tag publication therefore requires an authenticated Git push; it is not waiting
for owner authorization. This record does not assert that the remote tag exists.

No live external provider, actual TUI integration, power-loss behavior or broad
capacity/saturation result is claimed. The remaining remote tag operation must
be reported separately from the completed code publication.
