# Task 105: native multi-platform CI

## Scope and source

The owner requested Host CI like minicore-agent, with Runtime compiled into the
Host executable, and explicitly deferred Host/Runtime interaction changes.
Baseline: `dev` commit `6ea85fda737635e26b9d7cad17c3df1e54911bd9`.
Reference: minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739`.
Implementation topic: `ci/multiplatform-host`.

Version remains 0.9.20; Runtime remains 0.10.0 at
`3aa47629929906832dcb309afb7d242c1ae3e53f`. SPEC v4.93 retains SHA-256
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
No changes to Runtime interfaces, Turn/Step ownership, compaction responsibilities
or TUI are in scope. Existing historical evidence is preserved.

## Branch integration (2026-11-12, Asia/Shanghai)

The owner requested checking every Host branch and merging them into `dev` once
the multi-platform CI issue was resolved. The inspected destination was
`282a678b0ab7ee94ba774451619f640a62d9a050`.
[Its dev run 43071596301](https://github.com/zqcli/minicore-host/actions/runs/43071596301)
completed successfully at 2026-11-12 02:12:02 Asia/Shanghai: Linux quality,
all four native Clippy/test/doc-test jobs and all four package jobs passed.
Four non-expired package artifacts belong to this exact source. Windows also
passed the ordinary-user step; packages performed the native executable smoke
check and archive readback. This confirms the published dev checkpoint in
addition to the implementation-branch acceptance recorded below.

| Inspected branch | Exact tip | Integration resolution |
| --- | --- | --- |
| `ci/multiplatform-host` | `f29d3c9c7b7d2f10ed6bcc72a131ca3d85cd37a6` | Already an ancestor of dev; all implementation commits included. |
| `ci/windows-storage-probe` | `87b87201e2ba1f23cef233d71a59294dbcad539f` | Retain as a merge parent; keep the accepted dev files. |
| `ci/windows-job-exit-probe` | `eb1ec81248c42df90c7d51a40cfb9a9025ef710e` | Retain as a merge parent; keep the accepted dev files. |
| `main` | `b341b163e14543bfbe7f8d392ea243b4a123cdf7` | Already an ancestor of dev; its branch ref is unchanged. |

The two probe branches are completed investigation histories, not additional
Host implementations. The storage probe's Rust source is already present in
dev with identical blob `f0617cee9cd048dd7429792c872d2371231e78b6`. Its branch-only
workflow and initial user launcher are superseded by the accepted native CI
and `scripts/windows-user-tests.ps1`; the current task record includes its
experiments and outcomes. The Job probe deliberately has only three files and
deletes the rest of the repository relative to its parent. Applying those
experimental deletions would destroy the integrated Host. Its observations
and the resulting production completion barrier are already recorded below.

Use an explicit `ours` content resolution for both retired probe branches,
while retaining their exact tips as merge parents. Their original source,
including the standalone Job probe, remains reachable through dev history and
the immutable links below. Keep the accepted dev production/test/configuration
tree and update only `PROJECT.md` and this task record. No branch ref is deleted,
no history is rewritten, and no version or Runtime pin changes.

Local checks use `git merge-base --is-ancestor`, complete three-dot diffs,
blob-identity comparison, `git diff --check` and
`./scripts/check-whitespace.sh`. All branch tips must be ancestors of the
resulting merge; its first-parent diff must contain only the two records above.
An independent worker was requested with `gpt-6-astra` and `max` reasoning and
found no blocking issue in the native CI or this selective merge resolution;
these are requested settings, not an effective-runtime attestation.
The existing full native run validates the retained implementation. The new
merge's automatic CI is attached to that commit separately; its outcome is not
represented by the earlier run's results.

## Accepted native checkpoint (2026-11-11)

Verified source: `f29d3c9c7b7d2f10ed6bcc72a131ca3d85cd37a6`, tree
`a8d5817c9fef4e2802340b396588e481eea3ba41`.
[Run 43068086506](https://github.com/zqcli/minicore-host/actions/runs/43068086506) completed successfully at
17:42:38 UTC: the full Linux quality gate, all four native platform jobs and
all four release-package jobs passed.

| Platform | Release target | Unique Rust cases | Doctests | Package |
| --- | --- | ---: | ---: | --- |
| Linux x64 | `x86_64-unknown-linux-gnu` | 967 | 4 | [minicore-host-0.9.20-x86_64-unknown-linux-gnu.tar.gz](https://github.com/zqcli/minicore-host/actions/runs/43068086506/artifacts/13876809671) |
| macOS ARM64 | `aarch64-apple-darwin` | 960 | 4 | [minicore-host-0.9.20-aarch64-apple-darwin.tar.gz](https://github.com/zqcli/minicore-host/actions/runs/43068086506/artifacts/13878885250) |
| macOS x64 | `x86_64-apple-darwin` | 960 | 4 | [minicore-host-0.9.20-x86_64-apple-darwin.tar.gz](https://github.com/zqcli/minicore-host/actions/runs/43068086506/artifacts/13877936675) |
| Windows x64 | `x86_64-pc-windows-msvc` | 948 | 4 | [minicore-host-0.9.20-x86_64-pc-windows-msvc.zip](https://github.com/zqcli/minicore-host/actions/runs/43068086506/artifacts/13876988337) |

Each platform's authoritative `cargo test --workspace --all-targets --locked --
--list` contains 127 test targets. Counts use the actual target executable plus
case name, not nested subprocess output or the sum across operating systems.
All four native jobs pass strict Clippy, workspace tests and four doctests.
Linux also passes the 28 Python checks, rustfmt, strict rustdoc and committed/
working-tree whitespace checks through `./scripts/check.sh`.

The Windows job additionally executes five existing contracts as a real
non-administrator with its OS profile loaded and write access to the compiled
executables denied. All five selections execute exactly one passing case,
including real Host HTTP request/persistence and storage reopen/delete/identity
checks. The helper reports `StandardUserExitCode=0` and
`TemporaryAccountRemoved=true`. These re-executions do not add five unique cases.

Each native release log compiles the unchanged Runtime 0.10.0 revision into
Host 0.9.20. All four staged binaries pass actual `--help` with exit code zero,
empty protocol stdout and usage on stderr. Archive readback verifies 18 files,
one Host executable and its SHA-256 against that tested binary; Unix mode 0755
and absence of other executable files or links are checked. The archive hashes
and GitHub outer artifact ZIP hashes are different layers and are recorded
separately in [validation.json](validation.json). All four uploads succeeded.

Root integration review and an independently assigned final read-only review
of this exact source found no remaining P1/P2 findings in the task scope.
The review confirms the full packaging dependency gate, fixed Runtime library
dependency, unchanged SPEC and request/ownership production paths, and absence
of storage's development-only `test-support` feature from release builds.
The final Windows formatter correction changes no process or test behavior.

Publication targets `dev` by an ordinary, lease-protected fast-forward from
`6ea85fda737635e26b9d7cad17c3df1e54911bd9`, preserving all topic ancestry.
The commit containing these acceptance records is a documentation-only child
of the verified source: production/test code, manifests, lockfile, workflow and
scripts retain their verified blobs. Its normal automatic dev CI is a separate
run; the completed result above specifically identifies the tested parent.
No version change, tag, main update or Runtime interface change is part of this task.

The declared runners establish this native CI checkpoint. Windows storage still
requires local NTFS and the documented namespace permissions; the optional Bash
tool needs installed Git Bash on Windows. System libraries remain OS prerequisites.
No power-loss test, live external-provider trial or actual TUI acceptance is
claimed. Earlier pending statements below describe their historical checkpoints,
not outstanding blockers for this completed native run.

## Implemented scope

- Preserve the complete Linux quality gate and add native Linux x64, macOS ARM64,
  macOS x64 and Windows x64 Clippy/test/doc-test jobs.
- Gate four native release builds and archive uploads on all required checks.
- Build the existing Host Cargo package, including its pinned Runtime dependency.
- Package a real binary, initial configuration template and usable documentation;
  execute the native binary before publishing an artifact.
- Resolve platform compilation, filesystem and process-supervision prerequisites
  without substituting no-op saves, skipped platform tests or fake cleanup.

Three workers were requested with `gpt-6-astra` and `max` reasoning for disjoint
CI/packaging, storage and other platform-adapter scopes. These are requested
settings, not an independent runtime attestation. The root owns shared manifests,
the lockfile, top-level records, integration and publication.

## Initial validation and investigation

- Rust 1.90.0 `cargo check --workspace --all-targets`: passed on local Linux.
- `git diff --check`, workflow YAML parsing, Python helper compilation and
  official actionlint 1.7.12 validation: passed for the initial workflow.
- Cargo.lock adds only the 11 Windows dependencies required by process-wrap's
  existing pinned 8.2.2 Job Object feature; existing versions and Runtime pin
  remain fixed at this checkpoint.
- The original storage explicitly rejected non-Linux platforms. macOS reuses its
  Unix implementation; Windows needs actual native locking/path/sync semantics.
- An isolated standalone Windows API probe was published on
  `ci/windows-storage-probe`, commit
  `4ee7a94f2fe7c95cab65febe771c13fe3169b313`, with identical tree to local
  `e143ee4`. [Native run](https://github.com/zqcli/minicore-host/actions/runs/43029084962)
  passed: read/write directory handles completed real synchronization at all
  mutation phases; read-only and attributes-only handles failed with OS error 5.
  This runner used its administrator account. A separate non-administrator probe
  is pending. API success alone is not a power-loss durability test.
- The Windows adapter additionally pins winapi-util 0.1.11, windows-permissions
  0.2.4 and winsafe 0.0.29. Cargo adds six packages for these adapters without
  changing existing resolved versions. Windows requires real NTFS directory
  flushes, file identity and access checks; implementation is still in progress.
- Native process, workspace, diagnostic and executable integration targets pass
  on Linux after the portability changes. The Windows local-tools library also
  cross-checks successfully with `cargo check --locked --target
  x86_64-pc-windows-msvc -p minicore-host-local-tools --lib`; cross-checking is not
  native process execution.

## Native implementation checkpoint

The first CI source is remote `30a33abf64b20d298e0001822f4699df1e9362cf`, with
tree `44dee3ed5c85ec3bbc30151df3270fac19a366ac`, identical to local `2040f163`.
[Its native run](https://github.com/zqcli/minicore-host/actions/runs/43030627462)
passes both Linux gates. macOS ARM64 exposed five platform assumptions: FIFO
locking, two canonicalized temporary-path sync hooks, workspace root aliases
and Bash 3.2 variable-presence syntax. These have targeted fixes, preserving
real locking, fault injection and credential assertions. Windows was explicitly
blocked by the unfinished storage backend at that checkpoint.

The [standard-user Windows probe](https://github.com/zqcli/minicore-host/actions/runs/43031057105)
also passes on source `87b87201e2ba1f23cef233d71a59294dbcad539f` and confirms
`IsAdministrator=false`. Directory flushes work inside that user's profile;
opening `C:\Users` or the volume root for write is denied. The implementation
therefore uses the actual OS Known Folder Profile as an explicit existing
namespace boundary. It pins that directory before any Host mkdir, flushes the
profile and every descendant ancestor on acquisition (including failed earlier
creation attempts), and retains read-only no-delete handles above it. Roots
outside that profile still require every ancestor's real write/sync permission.
There is no PermissionDenied or filesystem-sync no-op fallback.

Windows storage now checks actual NTFS volume identity, owner/DACL permissions,
file IDs, hardlinks and reparse points. Directory share modes provide the root
lease and stable owned ancestry; journals retain actual OS file locks. Explicit
deletion drops the now-empty target guard before removing its parent entry.
Native confirmation of the full backend remains pending at this checkpoint.

Review found a concrete Windows cleanup defect in process-wrap 8.2.2 and released
10.0.1: their completion-port wait can accept a notification while descendants
remain alive. The dependency is now fixed to official source
[`2f4bc393d8bd03b4b1c81fee2e13ce82dd42906e`](https://github.com/watchexec/process-wrap/commit/2f4bc393d8bd03b4b1c81fee2e13ce82dd42906e),
version 10.0.1, which checks actual active-process accounting. A native Windows
regression keeps a real descendant alive after its leader exits and requires
wait to remain pending until that descendant is explicitly released. Private
adapter API changes pass the Linux process/app regressions and Windows cross
Clippy; Runtime's pin and interfaces are unchanged.

The second CI source is remote `271a18b3de034f9560fc1e4ece3cc30f3a476c16`,
tree `8e2bfc01dcf4b4ced81ec6f8e79491d6a19879b3`, identical to local `f3d3330f`.
[Its run](https://github.com/zqcli/minicore-host/actions/runs/43033621867) passes
both Linux gates. Windows compiles the production crates, then exposes
Unix-only pipe assumptions in transport tests. Shared test fixtures now use
actual Unix pipes or Windows named byte pipes, retaining real OS endpoint
lifetimes and backpressure. Tests previously excluded only for Unix pipes now
execute on Windows as well. The new ordinary-user CI step runs five existing
storage/app contracts and rejects missing, skipped or zero-test results.

Final review also replaces per-boundary Windows history hashing with a native
history lease that denies write sharing, including existing writable mappings.
The original byte locks, file-identity checks and uncertain-write prefix hashes
remain. Real mapping and child-process tests cover those lease boundaries;
the ordinary metadata path retains its original sharing. Windows atomic local
file replacements now preserve the source owner and effective DACL, including
protected/unprotected inheritance, before writing body bytes. Unsupported owner
changes or mismatched inherited ACLs are explicit failures.

Local Linux `scripts/check.sh` passes at the second checkpoint, including
965 unique Rust cases, 4 doctests and 28 Python checks. The native Linux release
build and packaging helper also pass: the archive contains one executable with
mode 0755, example configuration and documentation; `--help` succeeds on stderr
with empty stdout. `ldd` reports only system libraries, no Runtime shared object.
This local package is validation evidence, not a published final release.

The third CI source is remote `b24b712ccf1d159d8f53c3b37720c96438595f99`,
tree `93d44a68e1707302529eea95ddcc7df919eb9f5b`, identical to local `0983119c`.
Its clean local `scripts/check.sh` passes; the authoritative `--list` contains
965 Rust cases in 127 targets (the new Windows ACL target has no Linux cases),
with 4 doctests and 28 Python checks. [Native validation](https://github.com/zqcli/minicore-host/actions/runs/43036690189)
again passes both Linux gates. Windows Clippy exposes a fixture field used only
by the Unix alias path; Windows now creates a real directory symlink too.
Both macOS jobs retain one process-group cancellation failure: the actual child
is waited and both streams reach EOF, but a transient Darwin group probe returns
EPERM before the exited group is reaped. The adapter now continues its existing
bounded observation window, accepting only actual ESRCH as proof of absence.
A native regression preserves a real unreaped child to reject false completion.

Independent review also identified Windows named-pipe cancellation lifetime:
dropping a Tokio reader can precede actual IOCP handle release. The test fixture
now uses a receiving server endpoint and successful native disconnect as its
close barrier, propagating disconnect failure. It retains genuine OS pipes and
duplicate handles for both teardown orders.

Static review found historical tests reading or mutating history through a
second handle while its writer remains live. Windows correctly prevents this.
Storage's non-default `test-support` feature supplies explicit cross-crate
fixtures: actual read-only mappings, existing-owner write interruption and
existing-owner corruption. Only development dependencies enable the feature.
Production locks, actual accepted/pending state, byte serialization and original
range reconciliation are preserved; release builds do not enable these fixtures.

The storage/core/resources/transport all-targets Linux test run passed after
these fixture changes. This local run used `CARGO_INCREMENTAL=0`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0` and
`CARGO_BUILD_JOBS=2` after reusable incremental cache exhausted the scratch disk.
Repository profiles and source files were not removed. The execution environment
then became unavailable before the latest changes were committed. The subsequent
checkpoint is reconstructed from the published third source and reviewed edits,
and must pass its own native CI; the earlier local result is not treated as proof
of byte-for-byte identity with that reconstructed checkpoint.

## Reconstructed checkpoint and native verification

The reconstructed candidate is remote `9c5d0e6c24057c71993b78d4f87a9957bec78c79`,
tree `1f0bca3b297e33230e73975829cd19ffe3b5571a`. Its 43 changed files were
assembled from immutable Git trees with disjoint paths and no deleted files.
All four pipe-fixture blobs match their recorded pre-outage full hashes.
Other files were restored through checked source transformations; the candidate
is validated independently of the earlier local run.

[The fourth native run](https://github.com/zqcli/minicore-host/actions/runs/43042556299)
identified three rustfmt-only differences in reconstructed test files. The
follow-up applies exactly rustfmt's reported import order and panic formatting.
Windows strict Clippy also found helper/import usage hidden by old Unix-only
resource-test gates. Portable resource tests now use the actual Windows local
tools, while genuine FIFO and Linux process-inspection fixtures retain their
specific platform requirements.

Native Clippy keeps going across independent targets while still treating every
warning as an error. The native matrix additionally lists its actual test cases
with `cargo test --workspace --all-targets --locked -- --list`; nested subprocess
probe output is not used as an additional case count.

Package validation now reopens each completed archive, checks its exact file
set and single Host executable, verifies Unix modes and compares archived Host
bytes with the binary that passed the native smoke check. It prints both binary
and archive SHA-256 values.

The fourth run passes the complete Linux and Apple Silicon macOS native jobs.
The macOS log confirms both the new unreaped-group regression and the original
cancellation/Turn-finish cases pass. Intel macOS was still running when this
checkpoint was recorded.

The fifth candidate is `088bc6d6106f925c17e52e136acf3d7f63d0a630`,
tree `e24269d8f780395a6f07e5ab970fff51a3514c2e`.
[Its Windows Clippy job](https://github.com/zqcli/minicore-host/actions/runs/43043643222)
identified one remaining Unix permission fixture in owner diagnostics.
That test retains its real Unix directory-permission failure. On Windows it now
holds the actual Agent metadata file without delete sharing and verifies that
a real rename is denied. Session writes remain possible, while the Agent's
default-metadata atomic replacement fails at the original business boundary.
The accepted-work, dropped-waiter, receipt and diagnostic assertions are unchanged;
no production dependency or implementation changed in this correction.

The sixth candidate is `02b737ebe977d197caba6c00fd10bf4493ee9e2d`,
tree `a225cdc751101afe9a46c9266b9c55d3d1f092d8`.
[Its native run](https://github.com/zqcli/minicore-host/actions/runs/43044786811)
passes the Linux quality and native gates and Apple Silicon macOS. Authoritative
native listings contain 965 Linux and 958 Apple Silicon cases in 127 targets
each. Windows passes strict Clippy and proceeds to native tests. Intel macOS
exposes an actual cancellation race: the leader is waited, but its process group
still exists and both output pipes remain incomplete. The Turn correctly stays
in Finalizing with a cleanup fault. This is distinct from the already-covered
Darwin zombie/EPERM observation; no timeout increase or false cleanup confirmation
is accepted as its fix.

The Intel test-profile build in that run takes 16 minutes 52 seconds. Along with
the earlier exhausted local incremental cache, this motivates CI-only
`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0`. These reduce unused incremental/debug artifacts on
clean runners without changing optimization levels, debug assertions, overflow
checks, release profiles or local Cargo defaults. Their next native run remains
required; no measured CI speedup is claimed in advance.

The final packaging smoke executes the copied binary from the staged package
directory before archiving, then verifies the archive's bytes against the
original release binary. Independent review found no blocker in that change or
the CI-only profile settings. Python syntax, workflow parsing and whitespace
checks pass for those edits.

## Follow-up to the sixth native run

The Windows job passed strict Clippy, then exposed an actual root-lease sharing
conflict: initial `host.json` publication failed with OS error 32 because the
retained root handle denied the destination-directory write open used by an
absolute-path rename. The root now requests READ, WRITE and DELETE access while
sharing READ and WRITE only. The unshared DELETE request still excludes a second
root owner and pins the directory; ordinary metadata publication can obtain its
required parent write access. Identity-only reopens share DELETE and retain the
original guard. When the existing Profile itself is the root, the first Profile
open acquires this lease and its handle is cloned without a release/reopen gap.
The native regression keeps one root lease across initial publication,
replacement, identity checks and rejection of a competing root, then verifies
reacquisition after the owner is dropped. Real history locks and directory
synchronization are unchanged.

The Intel cancellation fix waits for the real leader, then concurrently drains
both streams, waits for the wrapper and confirms the process scope. During the
existing bounded observation window, a still-live Unix group receives another
SIGKILL; only actual ESRCH confirms its absence. The real leader's successful
exit status is retained even if the wrapper wait fails. A regression creates a
real surviving descendant after the leader has exited, then requires group
absence and EOF on both pipes. It does not claim deterministic reproduction of
the inferred kernel fork/signal snapshot race. The original completion criteria
and deadlines remain intact.

Windows process tests retain actual native process handles before cancellation
and require those same handles to be signaled when Job cleanup returns. Forced
Winsock teardown may produce exact WSAECONNRESET 10054 instead of EOF, but that
socket observation is not the evidence of process termination. The normal-exit
case still requires EOF. The Task 033 subprocess fixture now executes its own
test binary through a real stdin release gate on every platform. Resource prompt
fixtures compare complete prompts against actual canonical workspace paths.
EditJournal now uses the existing slash-separated relative-path formatter; the
original `nested/file` assertion is preserved and journal roundtrip coverage is
extended to a nested path.

The Windows job also exposed IPv6-to-IPv4 fallback against IPv4-only WSS fixture
listeners. A timed-out client could leave the fixture waiting forever in accept;
the job reached its 40-minute deadline before the standard-user step ran.
Fixtures now connect to the actual IPv4 listener address with a matching IP SAN,
retain certificate verification and original request budgets, and join both
fixture sides with cancellation and an explicit fixture-lifetime bound.
The diagnostic cleanup test observes real subscriber emissions and drains the
real writer before reading its log, preserving its live file lock. Independent
review additionally requires observing the final cleanup-blocked scheduling
event before shutdown, because publication of cleanup state or the preceding
error event alone does not prove that this trailing event has been emitted.
The forwarding subscriber's tracing-core dependency is test-only, uses the
already locked exact version with explicit std support, and adds no new package.

The latest local Linux regression groups pass: local-tools library/process
15 cases, owner-bound local tools 6 cases, and Task 033/resources 42 cases.
The transport group passes 19 cases; after the final trailing-emission correction,
that diagnostic case and all 5 edit-journal cases pass with `--locked` as well.
Workspace-wide strict all-targets Clippy, rustfmt, helper syntax and whitespace
checks pass. SPEC's full hash remains identical. Independent review finds no
remaining P1/P2 issue in the root-lease change, native process-handle probes,
WSS fixtures, test-only dependency edge, CI settings or staged smoke check.
These results do not establish native Windows support.
The next candidate must pass its own complete native matrix, including the five
standard-user Windows cases, before packages or the final dev publication.

## Seventh native checkpoint

Candidate `56a768c9175a6d7cfe7b5de986d9145049f5a727`, tree
`1fdf9abf4190b42fed59dd4a9492221c5d187ffb`, exactly matches the reviewed local
tree. [Native run 43055978426](https://github.com/zqcli/minicore-host/actions/runs/43055978426)
passes Linux quality, Linux native and Intel macOS. Authoritative native listings
contain 967 Linux cases and 960 Intel macOS cases in 127 targets each. The Intel
run passes both the real surviving-descendant regression and the original
owner-bound cancellation case. Apple Silicon was cancelled after remaining
unassigned to a runner, with no executed steps; this is not an executed test
failure or a passing platform result.

Windows strict Clippy passes and the native all-target test step completes rather
than hanging. The root-lease publication and WSS fixes remove the previous broad
failure groups. Four failures remain: one stopped persistent Session history
observation, two intermediate diagnostics-file reads, and a redirected native
descendant whose retained process handle is not yet signaled when the tool
reports cleanup. Standard-user tests and packages are consequently not run.

The history fixture now uses the existing actual-byte reader. A persistent
Session being Closed and its Agent being Stopped do not release its retained
journal; explicit deletion or controlled Host shutdown owns that retirement.
The test still verifies the original lifecycle, retained bytes and metadata.
The two diagnostic fixtures share a transparent test subscriber with the cleanup
fixture, observe actual emissions during the operation, and read the final real
JSONL only after the production writer reports Flushed. They preserve the
reentrant owner-query probes and strict final identity, ordering and privacy
assertions. Independent review finds no blocker. Local Task 051 passes 10 cases;
the three affected diagnostic targets pass, and transport all-targets strict
Clippy and workspace formatting pass.

The execution environment became unavailable before these five test files were
committed. They were reconstructed from the immutable seventh source and the
recorded exact diffs. Reconstructed blob hashes match the recorded diff prefixes;
the two rustfmt transformations are reapplied explicitly. The next native run
validates this checkpoint independently. No production change is included in
this fixture-recovery checkpoint.

## Native Job exit investigation

The process-handle mismatch remains a release blocker. A successful kill call,
pipe EOF or wrapper return is not substituted for the failing native assertion.
The isolated [Windows Job probe](https://github.com/zqcli/minicore-host/tree/d92f0537fe363a9644ab7a98f2934924d54365e1),
commit `d92f0537fe363a9644ab7a98f2934924d54365e1`, has script SHA-256
`5d3fab937d96596533dca5f869e9235333c5ff7b62b0bbef1e31a1e0a17f7a37`.
[Its native run](https://github.com/zqcli/minicore-host/actions/runs/43059602857)
completes 120 rounds on Windows Server 2025 build 26100 with real Git Bash and
native gated Python descendants. Every round first verifies exact Job membership
and retains the actual root/leaf process handles.

All 40 TerminateJobObject rounds observe ActiveProcesses zero and an empty Job
PID list while the same retained leaf handle still reports WAIT_TIMEOUT; it is
signaled later. Direct TerminateProcess and natural-exit controls, 40 rounds each,
do not observe this window. Those negative controls are observations, not a proof
that their accounting always implies process termination. The probe finally waits
for real signals and closes all owned handles. It is investigation code on a
separate branch, not a Host implementation or a platform acceptance gate.

Integrated source review confirms the Session execution/input/compaction/prompt
files, all compaction production sources and Runtime revision are unchanged.
The storage test-support feature has only development-dependency consumers and
is not enabled by the native release build. Independent workflow review confirms
all four native gates, actual standard-user identity/execution/cleanup checks,
and staged single-Host archive validation remain mandatory.

Final native gates, source identities, four-platform artifacts and dev publication
are pending. A configured matrix, cross-check or standalone probe does not
establish platform support.

## Eighth native checkpoint and completion barrier

Candidate `c89580739a80ac66a7fee9db8dbd2ed5e70c877a`, tree
`51777c9c0a9dbc0eabf3d60f60edf405c3699066`, contains only the five recovered
test files and their records. [Run 43061957432](https://github.com/zqcli/minicore-host/actions/runs/43061957432)
passes Linux quality/native and strict Clippy on all four platforms. Windows
has only the retained descendant-handle cleanup failure; the history and
diagnostic file-observation corrections pass. Apple Silicon exposes a diagnostic
fixture issuing explicit resume before its original owner finished returning.
Intel macOS has one large-history coverage fixture exceed its shared five-second
generation budget; its other native tests pass. Standard-user execution and
packages remain gated and were not run for this source.

The terminal diagnostic fixture now uses one current-thread executor and waits
for actual saving/model/preparation/tool-cleanup work to settle after the primary
error emission. In this fixed path, the final cleanup publication and clearing
the driver-running flag have no intervening await, so the observer cannot enter
that synchronous tail. The same work flags alone would not prove this across
threads. The original Turn and pending history save remain asserted, the original
inode is restored, and exactly one explicit resume continues that same save.
No production admission or ownership interface changes.

The large-history coverage test keeps all 80 source entries, its 71,500-byte
summary, and all coverage/order/tail/usage assertions. Only that non-performance
fixture receives a 60-second total generation deadline as a bounded hang guard.
The shared five-second context and dedicated 100-millisecond timeout-cancellation
test are unchanged; neither production settings nor per-call retry behavior change.

The second [Job exit probe](https://github.com/zqcli/minicore-host/tree/67a9a74fa4fdcef9a2232dac55bb809e70fe3994)
runs [160 real rounds](https://github.com/zqcli/minicore-host/actions/runs/43062636552).
All 40 Job-termination rounds again observe empty accounting before the held
leaf handle is signaled, and none delivers a leaf EXIT packet. In the 120 direct
termination/natural-exit/abnormal-status controls, the leaf EXIT packet itself is
observed before that handle is signaled. The abnormal-status mode calls
TerminateProcess with an exception status; it is not an actual access violation.
In all 160 rounds, receipt of the real Job ACTIVE_PROCESS_ZERO packet finds that
same held leaf handle signaled. Missing packets are recorded, never synthesized.

A [nested-Job probe](https://github.com/zqcli/minicore-host/tree/eb1ec81248c42df90c7d51a40cfb9a9025ef710e)
passes [20 native rounds](https://github.com/zqcli/minicore-host/actions/runs/43065942224),
covering child natural/forced exit with and without its own completion port.
While the real parent root remains gated and its Job remains active, the parent
port receives no ZERO; the child's own ZERO retains its child's key. After the
root is released, all parent ZERO packets have the parent's key and both held
process handles are signaled. The first nested fixture omitted SET_QUOTA from
its opened process rights and is not counted as evidence; the corrected source
requests the documented assignment rights. These probes support the documented
completion pattern on this runner, not a guarantee that every packet is delivered.

The private Windows adapter now keeps the root explicitly suspended, validates
its exact Job membership, associates a private completion port and starts an
owned monitor before resuming verified native thread handles. The monitor only
confirms a received ACTIVE_PROCESS_ZERO packet with the exact Job key and null
payload. Missing notifications, native errors and the existing one-second cleanup
observation deadline remain unconfirmed; a separate control channel ends the
monitor without posting fake completion. The monitor is actually joined, while
the existing uninterrupted process-wrapper wait and both pipe drains remain
independent requirements. Startup failure uses the retained real root handle
for stop and wait, including failure after native creation.

The new native regressions cover a still-suspended root at monitor readiness,
actual completion-port detachment without substituting the wrapper's zero
accounting result, and real root reclamation after deliberately missing Job
attachment. The existing same-handle descendant exit assertions are unchanged.
All Win32 unsafe calls are confined to one private Windows module with explicit
handle/pointer safety explanations; this crate otherwise denies unsafe code,
and other crates' policies are unchanged. The exact already-locked windows-sys
0.61.2 gains a direct Windows-only edge; no package version or Runtime pin changes.

Root and independent review cover the terminal fixture and fixed Windows adapter,
including a final startup-recovery correction: if TerminateProcess returns an
error after an earlier Job stop, the same held root is actually waited for within
the existing one-second observation window. Only a real signal confirms exit;
timeout or wait failure retains the original stop error. Root review also confirms
the large-history fixture-only budget change. These sources have not been run in
the unavailable local executor. Their integrated formatting, compilation, native
contracts, ordinary-user checks and package gates remain pending before platform
acceptance or dev publication.

## Ninth integration checkpoint

The integrated source is `77b7cb3b9b7461d8733cc15c5d7fcd699b93aa13`, tree
`915f7a9ee80798f572d7b2f52f201130c636b9a4`.
[Run 43067522158](https://github.com/zqcli/minicore-host/actions/runs/43067522158)
passes the 28 Python evidence checks, then rustfmt reports eleven formatting
hunks in the new private Windows module. The follow-up applies those exact
Rust 1.90.0 formatter changes; no process, timeout, ownership or test assertion
is altered by that formatting correction. Native jobs are still in progress
at this documentation checkpoint, and the quality failure correctly blocks
all package jobs.
