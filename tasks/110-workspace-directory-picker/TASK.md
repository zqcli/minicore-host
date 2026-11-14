# Task 110 — Host workspace directory picker

## Authorization and baseline

- Owner authorized separate minicore-web with its existing UI and native Host WSS;
  add `workspace.roots`, `workspace.list`, `workspace.mkdir`, then release Host 0.11.1.
- Host input: dev `32f074c57868533af81b0c6491440bae24b872c8` (0.11.0).
- Design first: SPEC 4.10.3, SHA-256
  `a9bec7b69a94082e7ea4d156f82f58361c241cb0d7ed9c01935f80b3bee9b792`.
- Mature directory reference: minicore-tui
  `2451d7289214385e3d21b46174b36f32a41dce45:src/remote/fs.rs`; its fixed
  MAX_ENTRIES quota is excluded under SPEC Rule 12.
- Requested engineering worker: gpt-6-astra, max reasoning. Effective runtime
  configuration is not independently attested.
- Follow-up authorization: fix the existing compaction declaration ordering
  defect exposed by the coordinator's real Web/Host integration.

## Local commits and ownership

1. `2dc0cd31a8116b65b5ace6d4c71d0a1c193413ee`
   `docs(spec): define Host workspace directory selection` — committed before code.
2. `c9cc848959fe8882d5925e46a00c43fb677f0efd`
   `feat(workspace): add Host directory picker operations` — source, tests,
   component documentation, configuration templates and workspace 0.11.1.
3. `06ab698ef4cc37d74984b8937b5284dbd5a94113`
   `fix(resources): sort compaction tool declarations` — existing contract fix
   and strengthened real-owner manual/automatic compaction regressions.

These are local Git identities. The coordinator owns remote publication, exact
local/public tree and commit mappings, independent integration and native CI.
No remote commit or tag is asserted by this worker record.

## Implemented behavior

Host configuration owns explicit `workspace_roots`; default empty exposes none.
Preparation validates and canonicalizes the directories, and existing Host reload
publishes them atomically with its other configuration. The directory scope only
applies to the three picker methods; Session workspace selection and tool policy
retain their existing boundaries.

Host owns authentication, current operator admission, accepted mkdir work and its
receipt. `host-workspace` performs actual filesystem operations; the shared typed
operation table supplies direct, stdio and WSS access without a Session. Roots/list
require a valid reader. Mkdir creates one child, preserves existing-entry and other
real errors, and requires the current operator. The Host retains the actual worker
through caller abandonment/disconnect and shutdown, storing creating/created/failed
facts in the existing receipt collection. Operation IDs remain correlation only.

Lists contain actual child directories and allowed symlink targets, with caller
page size and scoped cursors. They do not inherit the old fixed 1,000-entry limit.
Returned paths retain Host-platform semantics and reject lossy UTF-8 conversion.
Windows ordinary drive/UNC paths are simplified only when safe; volume GUID and
verbatim-dependent names retain their native prefix. Existing-name cursor checks
are separate from mkdir's Windows creation-name restrictions, so a returned cursor
can continue after an existing trailing-dot/space directory. The native Windows
fixture verifies that continuation and actual roots/list/mkdir path round trips.

Compaction now sorts its cloned tool declarations by name, matching Runtime's
existing strict request invariant. Session configuration order remains unchanged.
The two existing real Host + loopback HTTP/SSE compaction cases now select two tools
in reverse order and verify successful manual/automatic compaction, normal-request
ordering, and no utility tool execution or extra Turn-bound tool preparation.

There is no compatibility gateway, implicit root, application quota, retry,
generic operation manager, changed connection takeover rule or changed Turn
approval policy. Runtime ownership and TUI behavior are unchanged. Local path
containment retains the documented same-user concurrent path-replacement boundary;
this work does not introduce an OS filesystem sandbox.

## Local validation — Linux, Rust 1.90.0

Commands used `/home/agent/.cargo/bin/cargo`; build/test/lint commands used
`CARGO_BUILD_JOBS=4`. Component case counts below are run results, not a claimed
full-workspace unique case count.

| Command | Result |
| --- | --- |
| `cargo test -p minicore-host-protocol --all-targets --locked` | 79 passed, including all 55 catalog routes and strict directory receipt decoding |
| `cargo test -p minicore-host-workspace --all-targets --locked` | 38 passed: 7 directory, 5 edit journal, 8 existing filesystem, 18 Git/change cases |
| `cargo test -p minicore-host-core --lib --locked` | 135 passed, including 4 new directory authority/ownership/reload cases |
| `cargo test -p minicore-host-transport --test directory_picker --locked` | 2 passed, including real TLS WSS and OS-pipe stdio/direct parity |
| `cargo test -p minicore-host-resources --test task023 --locked compaction::` | 2 strengthened real-owner compaction cases passed after the ordering fix |
| `cargo test -p minicore-host-workspace --lib --locked` | Final affected rerun: 7 passed after Windows path/cursor corrections |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed; rerun after the path/cursor and compaction fixes |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` | Passed for all 13 packages |
| `cargo test --workspace --doc --locked` | 7 passed |
| `cargo build -p minicore-host --locked` | Passed; final binary rebuilt after the compaction fix |
| `cargo fmt --all -- --check` and `./scripts/check-whitespace.sh` | Passed |
| Three Python checks from `scripts/check.sh` | 28 passed: projection 15, RSS 5, performance helper 8 |

The directory transport fixture checks initialization-before-business, invalid-key
refusal, actual operator mkdir, read-only and unauthenticated mkdir refusal, actual
file effects, receipt readback, pagination and outside-root errors. The core fixture
checks expired callers, no automatic promotion, duplicate operation IDs, and a
controlled real write surviving waiter abort, disconnection and force shutdown.
The filesystem cases include 1,003 directories, symlink containment, real common
errors, Unix names/non-UTF-8 refusal and Windows drive/UNC/volume/verbatim formatting.

The initial version check found the exact host-storage protocol pin still at
0.11.0; it was updated. The new malformed-receipt test found that a Serde unit
variant accepted unrelated fields; creating is now a strict empty struct variant
with identical valid JSON. Full-target compilation exposed three complete older
HostConfig test fixtures; each now explicitly sets empty workspace roots.
Independent review found the Windows prefix issue; the safe conversion and existing
cursor-name correction are both included in the directory implementation commit.

All 13 workspace packages and internal pins are 0.11.1. A parsed Cargo.lock
comparison against the input commit confirms all 225 external package entries,
versions, sources and checksums are unchanged.

## Remaining coordinated validation and publication

The coordinator reports its independent real Web/Host WSS integration separately.
These Host checks use real local owners/filesystem/transports and controlled model
fixtures; they are not a live-provider or browser-rendering claim.

Full workspace/all-target execution and native platform gates are assigned to the
existing Host CI. The shared local disk has insufficient comfortable headroom for
another complete set of debug test binaries, and no other worker's output was
removed. In particular, the native Windows filesystem fixture is not claimed run
on Linux. CI must count unique cases with
`cargo test --workspace --all-targets --locked -- --list`; nested storage probe
output must not be counted twice. The coordinator will append public source mapping,
CI evidence and release/tag status after publication.
