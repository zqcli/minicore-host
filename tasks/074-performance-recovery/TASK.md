# Task 074: restore the performance harness without replacing lost evidence

## Status and scope

The four recovered Python scripts were preserved byte-for-byte in `93ef5adc`.
Following the independent Task073 SPEC decision relayed by the parent, a
separate narrow change reads the current concrete `agent.list` page. The 15
projection tests and five RSS tests pass after that change. This is source
recovery and parser validation, not a Host build, integration pass or new
performance result. These pure checks are now included in `scripts/check.sh`;
a complete run against recovered production is unverified.

- Fixed integration base: `f0900a7083f9f6ddddbe978cd6ba55f4b1405c50`.
- Isolated branch: `task/074-performance-recovery`.
- Worktree: `/workspace/shared/minicore-host-performance-recovery`.
- Original byte-preserving commit: `93ef5adc89353ae5e7b64e679da33a5bae8b4c95`.
- Initial recovery documentation: `1dfe42726886071a74f0a5c679d2ef888dc9fd97`.
- Concrete-page adaptation: `bd30418616db7f50df440b650e1b54ca220e687b`.
- Standard-gate integration: `4eaf777433e217e0d333fa144b407ddf771ea83e`.
- Own the four restored scripts, this task's recovery documentation and the
  separately authorized two-command addition to `scripts/check.sh`.
  Production, manifests, Cargo.lock, SPEC, workflow settings and shared project
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
copied file at `93ef5adc` passed `cmp` against that recovery archive. The later
parser adaptation intentionally changes two files; it does not change the
recovery archive or the original restoration commit.

Only the harness has an independently recorded historical whole-file digest
available in that conversation. The harness at `93ef5adc` is 799 lines /
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

The original restoration checks are recorded in
[pure-python-checks.txt](evidence/pure-python-checks.txt). Its four hashes are
preserved in [restored-script-sha256.txt](evidence/restored-script-sha256.txt);
that manifest applies to `93ef5adc`, not to the adapted working tree.

### Current adaptation hashes

At `bd304186`, `task058_perf.py` is 801 lines / 39,184 bytes. Only `agent_items`
and its projection tests changed. These are new source-integrity hashes, not
the original recovered file hashes:

- `scripts/task058_perf.py`:
  `75366c5b12f45aa82cfc0c849980085e57a9b4dade5e75e1e1d7625e4cb570fe`.
- `scripts/task059_projection_test.py`:
  `4b8d30524ede01124f89a9ef661b5de5cc00c28de87cf759842b258eabba8bfa`.

The RSS tests and summarizer retain their recovery hashes above. All four
current hashes are in
[current-script-sha256.txt](evidence/current-script-sha256.txt). The post-change
20-case check output is in
[current-page-python-checks.txt](evidence/current-page-python-checks.txt).
The two identical check commands were then added ahead of the Rust commands in
`scripts/check.sh`; no other gate or workflow setting changed. Their separate
execution and the shell syntax check are recorded in
[check-script-python-checks.txt](evidence/check-script-python-checks.txt).

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

This recovery and narrow adaptation retain the exact source guard,
fresh-output-directory guard, real production process/Runtime/HTTP workload,
explicit local SSE fixture,
body/identity/result assertions, one-reader subscription model, actual bash
cleanup checks, sampled-window CPU calculation and reviewed RSS union. It adds
no old-API fallback, threshold or benchmark framework.

## Static contract check at f0900a7

The old parser's concrete mismatch was reported before any adaptation:

- `crates/host-core/src/discovery.rs:38–42` defines `AgentListPage` with only
  top-level `items` and `next_cursor`. `Host::list_agents` constructs that value
  in `crates/host-core/src/host/discovery.rs`; the operation mapping at
  `crates/host-transport/src/operations.rs:281` returns it directly.
- The byte-preserved harness at `93ef5adc:scripts/task058_perf.py:388–400` requires
  outer `host_id` / `host_incarnation`, then `data.items` / `data.next_cursor`.
  Its first empty-Host inventory uses that path at line 473. The current flat
  page therefore cannot satisfy the restored parser.
- The parent relayed Task073's independent decision: SPEC section 8, line 1328,
  does not require a generic query wrapper for auxiliary summaries, including
  `agent.list` / `session.list`. Section 9, line 1469, requires identity and
  `data` for that chapter's state and session-content queries. The concrete
  production page is therefore retained; this is not a production fix.

The authorized adaptation at `bd304186` changes only `agent_items` and its
pure fixtures. It reads top-level `items` / `next_cursor`, requires the current
unpaged tail, validates each `AgentSummary.host_id` (`discovery.rs:55–57`),
and retains nonempty, unique, lexical Agent-ID checks. Missing page/item
fields, wrong Host IDs, nonlexical/duplicate/empty IDs, a bare list and the old
snapshot-only page are rejected. No alternate-shape fallback was added.
The harness does not call `session.list`.

The inspected `MessageSnapshot` (`query.rs:18–22`), `TurnResultSnapshot/Data`
(`query.rs:70–82`), kind-tagged history (`types.rs:203–216`), Session snapshot
(`types.rs:100–105`) and subscription envelope
(`host-protocol/src/observation.rs:59–68`) expose the fields read by the restored
harness. Their Chapter 9 identity/data checks, body-byte checks and Turn-result
assertions are unchanged. No second concrete shape mismatch was found in those
inspected paths. That limited static comparison does not establish runtime or full API
compatibility. SPEC's single-contract rule does not authorize a fallback.

## Commands and checks

The initial restoration used the following copy and comparison commands before
`93ef5adc` was committed. They describe that historical restoration step, not a
command to overwrite the later adaptation:

```sh
for f in task058_perf.py task058_rss_test.py task058_summarize.py task059_projection_test.py; do
  cp --preserve=mode "/workspace/shared/recovery-performance/scripts/$f" "scripts/$f"
  cmp "/workspace/shared/recovery-performance/scripts/$f" "scripts/$f"
done
```

Current source-only verification commands:

```sh
while read -r expected path; do
  actual=$(git show "93ef5adc89353ae5e7b64e679da33a5bae8b4c95:$path" | sha256sum | cut -d ' ' -f 1)
  test "$actual" = "$expected"
done < tasks/074-performance-recovery/evidence/restored-script-sha256.txt
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task059_projection_test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task058_rss_test.py
sha256sum -c tasks/074-performance-recovery/evidence/current-script-sha256.txt
scripts/check-whitespace.sh
bash -n scripts/check.sh
git diff --check
git diff --exit-code f0900a7 -- crates Cargo.toml Cargo.lock SPEC.md AGENTS.md .github
git diff f0900a7 -- scripts/check.sh
```

All 20 pure cases passed; they use explicit DTO fixtures and synthetic RSS
samples, not a fake Host measurement. Original script hashes at the recovery
commit, current script hashes, repository whitespace, shell syntax and unchanged production/manifest/
lock/workflow comparisons passed. The check-script diff contains only the two
authorized Python commands. These source/provenance checks are recorded in
[source-verification.txt](evidence/source-verification.txt). No Python bytecode
files were written. No Host process, SSE server, benchmark, binary build,
installation, full `scripts/check.sh` execution or Rust fmt/lint/test/doc gate
was run in this recovery stage. The parent owns the later combined full gate.

## Next step

The concrete-page decision is resolved; complete runtime validation is still
pending. Wait for the separate production logging/health recovery to finish.
The parent will provide a new fixed, fully validated production head before any
explicit release build or fresh measurement. Use new source IDs and result
directories for that work; do not bypass source guards, reuse lost evidence
names or call historical measurements current verification. Backup/publication
coordination belongs to the parent.
