# Saved-result history diagnostics verification

## Outcome

**No production change needed at fixed source
`50a1b2fedae73ce5c8b0a461a5bd6ed0edce68c8`.** The suspected missing
saved-only history-read ERROR is already covered. This task authors no fix and
makes no self-approval claim for a new implementation. It independently verifies
the pre-existing TASK087 code with actual failed file reads.

Only this report is added to the repository. No production file, permanent test,
manifest, dependency, Runtime source, SPEC, PROJECT or README is changed. There
is no cache, retry layer, new state, public event, push or main action.

Requested worker configuration: gpt-6-astra / max. Effective runtime settings
cannot be independently attested and are not represented as verified.

## Existing boundary

`crates/host-core/src/session/observation/query/saved.rs:36-49` already records
`session.history_read_failed` with phase `saved_turn_read` when its actual
`HistoryBackend::read_saved_turn` attempt returns SessionHistoryLoadFailed. Git
blame attributes this branch to existing integration commit
`163e904dbe59042fa901e4b95f99d93ba86e0bab`, not an uncommitted working-tree patch.

The backend maps actual cold-file load/validation errors to the registered
SESSION_HISTORY_LOAD_FAILED **3302**. The Session extracts only identity/Fault
metadata and its current revision under its lock, then emits after releasing
that lock. The original saved result/Fault still flows into the existing
retained-result, revocation and owner-visibility rechecks. These query precedence
rules and the backend's disk-only guard are unchanged.

An ordinary missing Turn is produced only after a successful read/projection as
SESSION_TURN_NOT_FOUND **3201**, so it does not enter the history-failure logging
branch. A loaded journal's validated opening records are not a second physical
read or a new diagnostic failure. No public-event reconstruction is involved.

## Independent real probe

Private evidence is `/workspace/shared/minicore-host-task095-evidence`;
checkout is `/workspace/shared/minicore-host-task095`, with its exclusive target
`/workspace/shared/minicore-host-task095-target`. It does not reuse the root's
running full-gate target or any other worktree's compilation artifacts.

`saved_history_probe.rs` uses real Host/resources, bound Runtime, the production
HTTP adapter, real persistent Store/Journal and actual stdio pipes. Only the one
loopback model response used to seed a completed Turn is scripted.

The probe stops and drops the original Host, opens a fresh cold metadata owner,
and performs the following without loading or resurrecting execution:

1. A valid saved result succeeds; an unknown Turn returns 3201.
2. Appending an invalid line after the valid saved terminal causes both a typed
   result query and an independent stdio result query to return original 3302.
3. Replacing history.jsonl with a directory causes a third actual read failure
   returning 3302. This is real filesystem behavior, not injected errno or a
   permission-denied assumption that could behave differently for root.
4. Restoring the file allows a later explicit query to succeed. A second unknown
   Turn still returns 3201. No resume, automatic retry or model call is added.
5. After flushing diagnostics, there are exactly **three total ERROR records**,
   one for each actual failed read. Every record is the existing Session history
   event with phase `saved_turn_read`, original Fault type/code/optional
   diagnostic_id and actual Session/Agent association. There is no ERROR for
   either normal miss and no duplicate main ERROR from stdio/RPC forwarding.
6. Complete public Session state remains unchanged, residency remains unloaded,
   no load/active Turn is fabricated, current Turn listing stays empty, and the
   query does not alter corrupt or restored history bytes. HTTP call count stays
   exactly one. TRACE diagnostics exclude private input/output/configuration,
   corrupt line content and the local history path.

The final probe passed once after compilation and ten additional fresh-process
repetitions. These are repetitions of one test, not extra unique tests.

An initial probe asserted numeric code 3300 instead of the registry's 3302. The
kind assertion already passed; this was solely an incorrect test expectation.
The original failure is retained as `probe-fixture-wrong-code.log`, not treated
as a production red. The early progress message's 3300 was also corrected.

## Commands and results

`env.sh` sets RUSTUP_HOME=/workspace/shared/rustup,
CARGO_HOME=/workspace/shared/cargo, PATH with its bin first,
TMPDIR=/workspace/shared/minicore-host-tmp, CARGO_BUILD_JOBS=2,
CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0, CARGO_INCREMENTAL=0,
and the exclusive CARGO_TARGET_DIR above.

```sh
cargo test -p minicore-host-transport --test diagnostic_storage --test task087_saved_result_contract --locked --offline
cargo test -p minicore-host-transport --test diagnostic_storage --no-run --locked --offline --message-format=json
cargo test -p minicore-host-core --lib saved --locked --offline
python3 /workspace/shared/minicore-host-task095-evidence/probe-build-run.py
cargo fmt --all -- --check
./scripts/check-whitespace.sh
git diff --exit-code 50a1b2fedae73ce5c8b0a461a5bd6ed0edce68c8 -- crates Cargo.toml Cargo.lock SPEC.md
```

All final commands pass. Existing diagnostic_storage: one test; saved-result
contract across typed/stdio/WSS: one test; core `saved` filter: eight tests,
including real saved-query revocation, loaded-publication and retirement races.
The full rustc command and exact selected Cargo artifact paths for the private
probe are in `probe.log`. Formatting and whitespace pass. Production diff is
empty. No complete-workspace inventory or total-workspace pass is claimed.

This documentation-only result does not repeat strict workspace Clippy/rustdoc,
all tests, Python checks, performance, live external-provider or TUI gates. The
parent's composed final gate remains separate. No new source needs an additional
implementation-fix review; any later source change requires its own review.

SPEC v4.93 SHA-256 remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
Cargo.lock SHA-256 is
`b8aa55600f443e7ff923096f37da833049d106078d5df44aa16362d70e11873b`.
Runtime is exactly `3aa47629929906832dcb309afb7d242c1ae3e53f`, version 0.10.0.
The inspected saved.rs SHA-256 is
`76795012b1aff6d175fdfa52acdd173e1bd98c081e4520c0b52c85eeb460d02d`.

## Evidence hashes

Paths are relative to the private evidence directory; `manifest.sha256` includes
scripts, selected artifacts, probe binary and the fixture correction log too.

| Evidence | SHA-256 |
| --- | --- |
| `baseline.log` | `5625ebb529ac22f12c6518db5383c7b804c8fffc0e7e69dc3af85eec91554c81` |
| `saved_history_probe.rs` | `3acbfc2d54a758a3a6d03b070869e05ad77b769847e751aba605e2e58e45d975` |
| `probe.log` | `c0a7aef056a79236b0a5ff3da2db16446ee1fb56946400c52f20771445a10621` |
| `probe-repeat.log` | `ef8fa22b19ab79e9d9e887ca2a09e06f212a95c38141e3f3321f0c0b3c47b008` |
| `final-checks.log` | `1a7d32a114d9bd3a7af3aa53542af83fc21b126223068bffab5dccc1847fdbf2` |
