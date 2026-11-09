# Implement durable Host storage

Status: completed for the storage foundation. Full-version integration and the explicit limits below remain with their actual owners.

## Scope and sources

- Implements the durable adapter for SPEC v4.93 §4, incremental record handoff in §5 and storage decisions in §15. `SPEC.md` is unchanged.
- Changed only `crates/host-storage/**` and this task record. Root workspace, Cargo.lock, PROJECT and protocol files belong to the parent.
- Runtime manifest baseline: `3fd5ecda1fb7c717ab0a39218d90268b4d97909c`. Final local checks used the clean Runtime patch worktree at `4f976b41f2381e2b09f3383aa66318b7aee2a8b3` and clean protocol worktree at `2c0b024f87a156bb1a1d47399cf302bc7262eedf`.
- Linux is the explicit supported platform. There is no weaker fallback for unsupported locking, filesystem synchronization or another OS.

## Functional commits

- `0c32443`: real directory-inode exclusive lock; fd-relative no-follow I/O; strict owned object envelopes; atomic file/directory synchronization; native incremental JSONL and exact-range reconciliation. Initial 21 tests passed.
- `936cdf3`: unknown/discarded native fields, real kernel EFBIG object/history failures, native tool and partial history, replacement and explicit deletion tests. 31 tests passed.
- `2366ff23c442f68040d7637596d95e3c99663da9`: all five focused-review corrections below, stable configuration/provenance checks and metadata-sync retry coverage. 43 code tests passed.
- `bf671a9ff2a4145fd6f39cc48cfd5fd7fb3e8e8f`: `AgentStore::load_record()` and direct-owner Session enumeration; healthy siblings initialize despite damaged default Session metadata or a missing default history. 44 code tests passed.
- `5746c2df8502b1978b39810879c189e21d4fb0df`: isolate canonical child IDs during enumeration while preserving actual read/write safety checks. 47 code tests passed.
- The final documentation commit adds the schema/platform contract, integration guidance, remaining limits and one compiling rustdoc example.

## Focused review and failure coverage

A read-only code/spec review found five issues. Each was corrected and regression-tested:

1. **Retry durability:** root/child directory ancestry and already-visible empty initial histories are explicitly synchronized again; existence never proves a previous failed sync completed.
2. **Replacement during I/O:** after write/sync, save confirmation rechecks current directory/file identity and exact expected length. Tests replace files/directories inside the synchronization window, including reconciliation.
3. **Terminal partial association:** one Loop's terminal partial forbids subsequent records for that Loop. Full response followed by its next-request partial remains valid.
4. **Numeric rounding:** parse raw JSON numeric tokens exactly before native decoding. Numbers the reused native codec cannot roundtrip are explicitly rejected, never converted silently to f64. Generic Host JSON retains exact large integers.
5. **Incremental cost:** validation uses a batch-local overlay and commits only touched indexes. No historical validation-map clone occurs per append; rejected batches leave accepted positions and old indexes unchanged.

Additional tests cover cross-process and same-process lock exclusion, symlink aliases, abnormal exit, stale markers, retained root guards, owner/path/hardlink checks, schema/corruption/duplicate failures, complete tails without newlines, missing histories, native tool-result associations and legitimate missing results, no-file memory modes, immutable creation provenance and exact confirmation ranges. Explicit test hooks inject zero writes, partial writes and file/directory sync failures around real files. Linux subprocesses induce actual EFBIG errors; their output uses pipes so redirected test logs do not inherit the test file-size limit.

Partial deletion is tested after history unlink but before required directory sync: the original closed metadata remains, so an explicit retry can finish. No rollback, recursion or automatic repair occurs. See the remaining limitation below for a later failure after metadata unlink.

## Final verification

Set the toolchain with:

`export RUSTUP_HOME=/workspace/shared/rustup CARGO_HOME=/workspace/shared/cargo PATH=/workspace/shared/cargo/bin:$PATH`

Exact commands, all passed against the final working tree:

- `cargo fmt --manifest-path crates/host-storage/Cargo.toml --check`
- `cargo clippy --manifest-path crates/host-storage/Cargo.toml --all-targets --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-handoff"' -- -D warnings`
- `cargo test --manifest-path crates/host-storage/Cargo.toml --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-handoff"'`
- `RUSTDOCFLAGS='-D warnings' cargo doc --manifest-path crates/host-storage/Cargo.toml --no-deps --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-handoff"'`

Final results after the concurrent-metadata follow-up: **19 unit tests + 37 integration tests + 1 compiling rustdoc example** passed. Formatting, Clippy warnings-as-errors and rustdoc warnings-as-errors passed. Standalone test Cargo.lock/target files and the temporary protocol symlink are not committed. The parent integrates the reproducible workspace lock and vendored Runtime patch, then runs the full workspace gates.

## Integration rules and remaining limits

- All disk methods run in owned blocking work outside business-state locks. Accepted workers retain their journals/root guards until actual completion. Failed saves preserve accepted positions and the exact pending range; no retry re-executes Runtime or tools.
- Initialize Agents with `load_record()`, then initialize children independently and retain per-Session faults. Strict `load()` additionally checks the default. An unavailable default stays unavailable; it is never redirected. Clear and save the Agent pointer before committing closure of its current default Session.
- Memory objects create no own records, histories or result stores. No quotas, TTLs, retention budgets, migration, tail repair or execution recovery are introduced.
- **Remaining explicit deletion limitation:** if `session.json` has already been removed and later directory cleanup/sync fails, the convenience deletion API does not reconstruct authority from an empty directory or declare completion. A business-owner phased-deletion handle that retains the accepted operation's exact partial facts is not implemented here. This is a remaining full-version lifecycle task, not a successful rollback or recovery.
- Standalone Summary records preserve native summary content; they do not imply prefix replacement. Actual compaction replacement needs its owner-level commit contract.
- No power-loss experiment, network-filesystem certification, other-OS validation, live provider execution or TUI validation was performed by this storage task. The focused read-only review does not replace the user's separate independent review.
- No remote push or tag was attempted.


## Follow-up: isolate unsafe child entries during enumeration

- Persistent-owner composition review of baseline `3945488` exposed an additional boundary error: an otherwise canonical Session ID entry made writable by others caused `list_ids` to reject the whole Agent before the Session owner could record its own fault. The same helper affected Host-level Agent enumeration.
- Added regressions first: both the Session and Agent child-isolation tests failed on real chmod 0777 directories before the production change.
- Enumeration now returns canonical direct-child IDs without opening child paths. Each actual child load/write retains every existing no-follow, directory permission, file type, schema and ownership check. No symlink target is traversed during enumeration.
- The two isolation tests cover writable directories, regular-file replacements and symlinks to otherwise valid objects. In every case the bad child remains explicitly unreadable/unwritable, original data stays unchanged and a healthy sibling can load and complete a durable append.
- A third regression verifies that unsafe shared `agents` or `sessions` collection containers still fail at their actual owner boundary. Invalid names without a canonical object ID remain structural errors; they are not silently skipped.
- Focused pre-fix command: the exact test command above with `unsafe_child_ -- --nocapture` appended; 2 tests failed as expected. Post-fix focused commands with `unsafe_child_ -- --nocapture` and `unsafe_shared_collection_ -- --nocapture` passed (2 and 1 tests respectively).
- Full follow-up gates use the exact fmt, Clippy, test and rustdoc commands above. Result: 11 unit + 36 integration + 1 compiling doc example passed; no warnings, remote writes or scope changes.

## Follow-up: coordinate metadata snapshots with same-instance publication

- Persistent-owner stress review found a real Agent/default Session concurrency race: their separate saves legitimately replace their own JSON records while the other owner's parent/default check has just opened the old inode. `nlink == 0` then incorrectly looked like an unsafe file. The new storage regression reproduced this before the fix on its first run in 1.42 seconds, without sleeps: 512 Agent updates, 512 default Session updates and concurrent reads.
- Object JSON reads now take a per-file shared snapshot gate; atomic publication takes that same path's exclusive gate for target validation, rename, sync and confirmation. The root's shared index only locates or retires live gates, never performs I/O or waits on a file gate while holding its mutex. Typed parsing and parent/default traversal occur after the raw snapshot gate is released. No owner-state lock, cross-object lock chain, global Session serialization or public API was introduced.
- A same-instance atomic publisher cannot unlink a snapshot's inode before validation finishes. The existing `nlink == 1`, no-follow, type, ownership and permission requirements remain strict. Post-read file and attached-directory checks conservatively reject observed external replacement, movement, unlink, extra hardlink, unsafe permissions, symlink or detached hierarchy. No automatic retry, metadata rewrite or history rule relaxation is used. Append-only history remains outside these metadata gates.
- Readable snapshots do not confirm failed saves. A real-file test injects the directory-sync failure after publication, verifies the save still fails, and verifies the following read neither retries synchronization nor reports persisted. Same-object accepted save ordering still belongs to its direct owner; the gate is not a revision compare-and-swap or a multi-object transaction.
- Deterministic tests pause a live snapshot, show its same-path publisher cannot rename yet, and complete a real save on an independent path. Additional tests cover gate identity, waiting operations, poisoned synchronization, concurrent final lease retirement and no retained dead path entries. Focused read-only review caught the first retirement implementation's simultaneous-last-drop issue; it was corrected by releasing the strong reference under the index mutex, regression-tested and re-reviewed with no remaining blocking findings.
- Initial compilation was blocked by a full `/tmp` tmpfs (`ENOSPC`), not a source failure. Remaining gates used `mkdir -p /workspace/shared/minicore-storage-tmp` and `export TMPDIR=/workspace/shared/minicore-storage-tmp` before the exact toolchain and commands above. Other workers' files were not deleted.
- The focused regression command is the full `cargo test` command above followed by `concurrent_parent_child_metadata_saves_preserve_confirmed_records -- --exact`. It passed 10 consecutive runs after the fix, exercising 10,240 successful metadata saves plus concurrent parent/default reads and reopen validation. This is real temporary-file testing, not power-loss or same-OS-user adversary certification.
- Final full gates: 19 unit + 37 integration + 1 compiling doc example; fmt, Clippy `-D warnings`, rustdoc `-D warnings`, and `git diff --check` passed. No root manifests, owner code, protocol code, SPEC contents or remote refs changed.

## Integrated file-gate reacquisition regression

An additional independent-review test now races eight threads through 512 acquire/write/drop cycles each on one metadata path. It checks that simultaneous writers never share the path and that the weak registry is empty after all leases retire. It adds no production synchronization or new limit. The integrated full workspace gate passes with 20 storage unit tests, 37 storage integration tests and the compiling storage doctest, together with all other workspace tests and formatting/Clippy/rustdoc checks. Production safety checks and append-only history rules remain unchanged.
