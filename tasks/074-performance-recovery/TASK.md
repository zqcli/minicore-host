# Task 074: restore the performance harness without replacing lost evidence

## Status and scope

The four recovered Python scripts are restored byte-for-byte. The 15 projection
tests and five RSS tests pass as pure Python checks. This is source recovery,
not a Host build, integration pass or new performance result. A known
`agent.list` wire-shape mismatch at the fixed integration base remains under
independent owner/SPEC review; the restored harness is not claimed runnable
against that base.

- Fixed integration base: `f0900a7083f9f6ddddbe978cd6ba55f4b1405c50`.
- Isolated branch: `task/074-performance-recovery`.
- Worktree: `/workspace/shared/minicore-host-performance-recovery`.
- Code-only commit: `93ef5adc89353ae5e7b64e679da33a5bae8b4c95`.
- Own only the four restored scripts and this task's recovery documentation.
  Production, manifests, Cargo.lock, SPEC, `scripts/check.sh` and shared project
  documentation are unchanged. No publication, push or main merge was performed.
- Requested worker configuration: `gpt-6-astra` / `max`, continuing the existing
  task. No configuration change was made; effective runtime settings are not
  independently verified.

Read `AGENTS.md` and the relevant SPEC ownership, query/observation and performance
contracts. SPEC v4.93 retains SHA-256
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
In particular, SPEC lines 2302–2310 require distinct stock/queue/concurrency/
subscription workloads, actual versions and methods, and unmeasured limits;
test counts must not become a capacity claim.

## Recovery source and byte identity

Input archive: `/workspace/shared/recovery-performance/scripts/`. It was
reconstructed from complete source reads and exact patches visible in the
original performance task's conversation after the execution environment was
replaced. No conflicting destination file existed at the fixed base. Each
copied file passes `cmp` against that recovery archive.

Only the harness has an independently recorded historical whole-file digest
available in that conversation. The current recovered harness is 799 lines /
39,090 bytes and matches that digest exactly:

- `scripts/task058_perf.py`:
  `dcc7ca992b98076d166ee7fe59a56450b332b1c5692a92d0f546be5a97beb59c`.

The following are **new recovery-integrity hashes**, not claims that an old
whole-file checksum was independently recovered:

- `scripts/task059_projection_test.py` (15 pure tests):
  `4ffdaa69e6b59d4e727dea2fb1729660f1989d43eca1463a4e9afe265fbb4548`.
- `scripts/task058_rss_test.py` (five pure tests):
  `447ff75bf8a43bc99b9e6eab8deadac804fa8b5655ee7597e71048c09a3724a1`.
- `scripts/task058_summarize.py`:
  `19ba3e056059a8308bfc0bf0986402de85d288b99769cb74285e488023ac85d2`.

The source-only checks are recorded in
[pure-python-checks.txt](evidence/pure-python-checks.txt). All four present script
hashes are listed in [restored-script-sha256.txt](evidence/restored-script-sha256.txt).

## Lost evidence is not reconstructed from summaries

The original Task058 three JSONL runs, Task059 two adaptation-smoke JSONL runs,
Task059 three final JSONL runs, all three corresponding raw `results.json`
files, original executables/targets and complete final checksum manifest remain
missing. Original full build/environment/resource traces and complete original
gate evidence are not recovered by this task. Historical summaries and reported
gate outcomes cannot replace those files or certify current code.

The old `b291551` source/binary and `c306ade`/`fe8bac7` evidence/documentation
identifiers remain historical references only. No fake Git object, executable,
raw sample or old-source result is generated. The archive's
`historical-transcripts` directory is deliberately not copied into this commit
or labeled as new measurement evidence. Only the four script files are imported.

This restoration retains the exact source guard, fresh-output-directory guard,
real production process/Runtime/HTTP workload, explicit local SSE fixture,
body/identity/result assertions, one-reader subscription model, actual bash
cleanup checks, sampled-window CPU calculation and reviewed RSS union. It adds
no old-API fallback, threshold or benchmark framework.

## Static contract check at f0900a7

One concrete mismatch was reported before changing any contract:

- `crates/host-core/src/discovery.rs:38–42` defines `AgentListPage` with only
  top-level `items` and `next_cursor`. `Host::list_agents` constructs that value
  in `crates/host-core/src/host/discovery.rs`; the operation mapping at
  `crates/host-transport/src/operations.rs:281` returns it directly.
- The byte-preserved harness at `scripts/task058_perf.py:388–400` requires
  outer `host_id` / `host_incarnation`, then `data.items` / `data.next_cursor`.
  Its first empty-Host inventory uses that path at line 473. The current flat
  page therefore cannot satisfy the restored parser.
- This is an observed shape mismatch, not a resolved SPEC judgment. The parent
  assigned independent owner review to determine the correct contract. Neither
  production nor the harness was changed to force compatibility. No Host was
  run to rediscover the statically evident mismatch.

The inspected `MessageSnapshot` (`query.rs:18–22`), `TurnResultSnapshot/Data`
(`query.rs:70–82`), kind-tagged history (`types.rs:203–216`), Session snapshot
(`types.rs:100–105`) and subscription envelope
(`host-protocol/src/observation.rs:59–68`) expose the fields read by the restored
harness. No second concrete shape mismatch was found in those inspected paths.
That limited static comparison does not establish runtime or full API
compatibility. SPEC's single-contract rule does not authorize a fallback.

## Commands and checks

The four files were copied with their existing modes and compared byte-for-byte:

```sh
for f in task058_perf.py task058_rss_test.py task058_summarize.py task059_projection_test.py; do
  cp --preserve=mode "/workspace/shared/recovery-performance/scripts/$f" "scripts/$f"
  cmp "/workspace/shared/recovery-performance/scripts/$f" "scripts/$f"
done
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task059_projection_test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task058_rss_test.py
sha256sum -c tasks/074-performance-recovery/evidence/restored-script-sha256.txt
scripts/check-whitespace.sh
git diff --exit-code f0900a7 -- crates Cargo.toml Cargo.lock SPEC.md AGENTS.md scripts/check.sh
```

All 20 pure cases passed; they use explicit DTO fixtures and synthetic RSS
samples, not a fake Host measurement. Repository whitespace checks and unchanged
production/manifest/lock/check-script comparisons passed. No Python bytecode
files were written. No Host process, SSE server, benchmark, binary build,
installation or Rust fmt/lint/test/doc gate was run in this recovery stage.

## Next step

Keep these bytes frozen while independent review resolves the `agent.list`
contract and the separate production logging/health recovery finishes. The
parent will provide a new fixed, fully validated production head before any
explicit release build or fresh measurement. Use new source IDs and result
directories for that work; do not bypass source guards, reuse lost evidence
names or call historical measurements current verification. Backup/publication
coordination belongs to the parent.
