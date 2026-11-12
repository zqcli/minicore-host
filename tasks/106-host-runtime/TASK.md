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
4. Run focused regression checks, the full project gate, native CI and release
   package verification; independently review the integrated source.
5. Publish reviewed commits to dev without rewriting history, then create the
   requested release tag on the verified commit.

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
  workspace packages to 0.10.1. Standalone API replacement follows this import.
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
  than a history slice boundary. The corresponding regression checks are being
  integrated before the complete workspace gate.
- Prepared reproducible release probes for same-Turn continuation and long
  history, plus compaction calculation. Baseline correctness passes; timings
  collected while builds run are explicitly excluded from performance evidence.

## Validation and publication

Pending. Record exact source commits, commands, outcomes, limitations, native CI
runs, test counts, code-size comparison, performance evidence and final tag here.
