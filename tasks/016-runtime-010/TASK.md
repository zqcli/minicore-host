# Task 016: Adopt Runtime 0.10

## Original isolated scope and baseline

- Host baseline: `8079970d3d7b35912ae4b45118d8e980d5f3e749`.
- Branch/worktree: `topic/runtime-010-integration` at `/workspace/shared/minicore-host-runtime-010`.
- Tested Runtime: version `0.10.0`, local commit `cd59b6ecbad5ccfb67552660290992662253219b`, tree `a9affb5ec0a2c40dc68c77c1e0dec1a13bd2cc49`, directly based on published `18a6ac9b82d003330cd870275efca793cdcc9a96`.
- The candidate is not treated as remotely published. Source commits require the complete migration and a separately finalized dependency pin before integration. The committed manifests and lockfile remain unchanged from the Host baseline.
- Local work only: no Host main/dev change, sibling integration, Runtime edit, Agent/TUI edit, remote push or main merge.
- Read `AGENTS.md`, unchanged `SPEC.md` Rule 12, Runtime's migration notes and `scripts/check.sh`. Specification SHA-256 remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

## Source commits

- `c676f5f7bab127830da7025cbe5d0d5a01747772`: mechanical `Text` and infallible model-event constructor adoption across production diagnostics and test adapters.
- `8a26e318edce5d3429ba4ac5e81afee36d7e42c2`: remove the provider's 65,536-byte accumulated argument cap, 262,144-byte HTTP error-body cutoff and obsolete UTF-8 event splitting. Keep typed ID/name grammar, object/JSON validation, cancellation and original deadlines.
- `4a262aa7376ee2b1e0c25e04c608cc8c4c7e72b1`: describe invalid Session input as empty, without the removed Runtime text-limit explanation.
- `4edc90e22e04c66c7e0cd91ed7ea5d5802c9ace0`: seven new provider/Runtime tests and two new Host-owner HTTP tests; rename the existing event-size test to avoid claiming obsolete splitting.
- `0c93ca0a3791686a4eefef7348389fb6e5c0b92b`: remove test-only comparison clones flagged by warnings-denied Clippy.

These are review units, not claims that the old Runtime pin builds each intermediate source commit. Session production/test ownership was handed to the observation worker after the two narrow Session migrations; later work here changes only provider/independent HTTP tests and this task log. Parent integration owns aggregate `PROJECT.md` and dependency-pin updates.

## Behavior and regression coverage

All new provider cases traverse real loopback TCP, reqwest, SSE and the actual Runtime, using explicitly scripted HTTP replies rather than a substitute Runtime or live model service.

- Text and reasoning deltas larger than 256 KiB remain one adapter event each and become complete exact Runtime assistant content.
- Tool arguments/results larger than 256 KiB, including a 5,000-element JSON array, execute once and replay the original call identity, arguments and result in the next actual HTTP request.
- Large malformed arguments, non-object JSON and unsafe 128-container nesting never execute or fabricate assistant/tool completion; raw argument fragments remain exact partials.
- A 65-container argument succeeds, proving removal of the old depth-32 quota without bypassing the parser stack guard.
- Large incomplete tool headers retain exact provider fragments in opaque partial replay without invented executable calls.
- Large HTTP error bodies still distinguish recognized transient errors, hard quota errors and unknown errors. Their free text stays out of diagnostics, response retry headers remain lower bounds, invalid JSON grants no retry and a 60-second header is not shortened to Runtime's 30-second retry-wait maximum.
- Real Host -> Agent -> SessionController -> Runtime -> HTTP accepts input and role text above 256 KiB, retains a large complete response in history and sends it unchanged on the next Turn. Empty input remains invalid.
- Real Host ownership retains a large truncated HTTP response only as a partial and reports failed execution, without assistant-history fabrication or retry.
- Existing model-token/capability checks, cancellation/deadline cases, all owner history/save and dual-failure regressions, and the P2 HTTP-200 SSE `Retry-After: 60` regression remain in the full passing suite.

## Explicit temporary validation setup

All checks used this environment (the debug/incremental overrides reduce generated disk artifacts only):

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
export TMPDIR=/workspace/shared/minicore-host-tmp
export CARGO_TARGET_DIR=/workspace/shared/minicore-host-runtime-010-target
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
```

For validation only, the four existing Runtime dependency requirements in
`crates/{host-core,host-provider,host-storage,host-transport}/Cargo.toml` were
changed to `version = "=0.10.0"` while retaining their original Git revision.
The following external command-line patch, supplied after each Cargo subcommand,
selects the real local source instead of attempting to publish or fetch that
candidate. Cargo regenerated a temporary lockfile. These five tracked files
were restored after validation; none of their temporary edits is committed.

```sh
cargo metadata --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-unbounded"' \
  --offline --locked --format-version 1 --filter-platform x86_64-unknown-linux-gnu
```

Verified Runtime metadata:

```text
name: minicore-runtime
version: 0.10.0
source: null
manifest_path: /workspace/shared/minicore-runtime-unbounded/Cargo.toml
id: path+file:///workspace/shared/minicore-runtime-unbounded#minicore-runtime@0.10.0
```

No `.cargo` patch, vendor copy, bootstrap source or fake Runtime was created or committed. Unfiltered offline metadata initially requested an uncached Windows-only `winapi-util` package; the explicit Linux filter verifies the platform actually tested. Windows/macOS execution is not claimed.

## Commands and results

For the exact repository gate, this exported shell wrapper applies only the
external local-source selection and offline cache use. Putting the patch after
the subcommand also reaches the external Clippy command.

```sh
cargo() {
  local subcommand="$1"
  shift
  if [[ "$subcommand" == fmt ]]; then
    command cargo fmt "$@"
  else
    command cargo "$subcommand" --config 'patch."https://github.com/zqcli/minicore-runtime".minicore-runtime.path="/workspace/shared/minicore-runtime-unbounded"' --offline "$@"
  fi
}
export -f cargo
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
```

- `cargo check --workspace --all-targets --locked --offline` with the explicit patch: passed.
- Full `scripts/check.sh`: passed whitespace, formatting, workspace/all-target Clippy `-D warnings`, workspace/all-target tests, **3 doctests** and `RUSTDOCFLAGS=-D warnings` workspace documentation.
- Listing contains **299 unique unit/integration cases**, counting one `: test` line per listed test. The storage child-process probe's nested execution is not an additional case.
- Focused command: `cargo test -p minicore-host-provider --test http_runtime -p minicore-host-core --test http_host --locked`. All **38 provider HTTP tests and 3 Host HTTP tests** pass. Five repeated runs of that command also pass, 41 cases per run.
- Two temporary production mutations independently reinstated only the old argument cap or old error-body cutoff against real Runtime 0.10. The corresponding new exact tests each failed with a test assertion (exit 101), proving the regressions detect those defects. Each original source file was restored immediately and verified with `git diff --exit-code -- crates/host-provider/src`; the focused repetitions and final full gate use the restored source. No mutant was committed.
- An initial full-gate attempt passed Clippy but failed test linking due to actual shared-disk exhaustion (`No space left on device`), not a failed test. Only this worktree's generated target was cleaned here; the successful fresh build disables debug/incremental artifacts. Parent separately reclaimed finished build targets. The failed attempt is not counted as a passed gate.
- The first Clippy pass found two avoidable test clones, corrected by `0c93ca0`; the subsequent complete gate passed. An earlier wrapper placement failed dependency resolution before testing and was corrected to the command shown above.

Local logs: `/tmp/host-runtime-010-full-gate.log`, `/tmp/host-runtime-010-test-list.log`, `/tmp/host-runtime-010-legacy-{arguments,error-body}-red.log`, `/tmp/host-runtime-010-http-repeat-{1..5}.log`. These are local evidence, not remote CI results.

## Remaining integration boundary

Parent must obtain an actually published Runtime 0.10 revision, update all four
Git dependencies/version requirements and regenerate the final lockfile in a
separate integration commit, verify metadata uses that Git revision, and rerun
the full integrated gate without a local patch. The source-only staging branch
still contains the old committed Runtime pin and is not independently buildable
until that integration step. No remote publication, independent Host review,
live provider use, TUI validation or completed first-version release is claimed.

## Integrated published checkpoint

The original staging limits above are historical. Runtime publication completed
on the existing `feat/host-history-handoff` branch at
`59ee8ce668f72a21d408e6247d3ce075b260db14`, with the exact tree of local
`cd59b6e`. All seven exact-head CI jobs passed. The included
`docs/runtime-publication.json` records the seven old/new commit identities,
verified trees/messages/authors and CI.

Source-to-dev integration: `c676f5f` -> `f0531ab`, `8a26e31` -> `8657f85`,
`4a262aa` -> `0bc0e26`, `4edc90e` -> `c955c44`, `0c93ca0` -> `bb3e0d2`,
`5667435` -> `a1d6224`. Commit
`275110695551c30582d1302e6b01ab23a3aad536` pins all four Runtime manifests to
exact `=0.10.0` and Git revision `59ee8ce`, with the matching lockfile. One new
HTTP-owner assertion adopts the independently integrated `Fault::kind()` Option
API. No local source override is needed or present.

Actual integration commands, using the environment above but the main
checkout's own target directory and **without the temporary patch**:

```sh
cargo metadata --locked --format-version 1 --filter-platform x86_64-unknown-linux-gnu
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
```

Metadata reports exactly one Runtime package, version `0.10.0`, with source
`git+https://github.com/zqcli/minicore-runtime?rev=59ee8ce668f72a21d408e6247d3ce075b260db14#59ee8ce668f72a21d408e6247d3ce075b260db14`.
The full gate passes formatting, committed/staged/working whitespace, strict
Clippy, **340 unique tests**, **3 doctests** and strict rustdoc. The unique
count was verified by listing; the difference from the topic's 299 is the
independently integrated workspace/WSS/client-review work.

An initial pin-preparation command updated only two version requirements and
failed resolution before testing; all four were then made exact. The first
compile caught the new test's old Fault assertion, corrected in the pin commit.
Neither failed attempt is reported as passing. Local evidence files are
`runtime-010-integrated-2751106.log`, `runtime-010-test-list-2751106.txt` and
`runtime-010-remote-metadata.json` in the validation directory.

The separate authoritative retry-progress Runtime extension is not part of this
published pin. Host production-process/TUI/live-model checks and full external
review remain pending.
