# Task 093: independent Session handoff, stop and cleanup review

## Fixed checkpoint and verdict

- **APPROVE for the three scopes below** at composed Host commit
  `8b129d4bdd35e223f8efe80059c4c8ea7bfe1df6`, tree
  `026d5a8fa3cb2396cf02a1ac1dfe62a3a89f668c`. No unresolved production
  finding was reproduced in this review. This is not approval of all task053,
  the full first version, or later commits.
- Requested reviewer configuration: `gpt-6-astra`, reasoning `max`.
  Effective runtime settings were not independently inspected or claimed.
- Independent worktree: `/workspace/shared/minicore-host-task093-review/target`.
  Production files remained read-only. Only this task report is committed;
  independent probe sources and raw output are retained as private evidence.
- Baseline: `AGENTS.md` and SPEC v4.93, especially section 12. SPEC SHA-256
  remains `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
  Cargo.lock SHA-256 remains
  `b8aa55600f443e7ff923096f37da833049d106078d5df44aa16362d70e11873b`.
- Runtime remains the actual published `3aa47629929906832dcb309afb7d242c1ae3e53f`
  dependency, not a local override. Agent is a code reference, not a behavior
  baseline. TUI is untouched: the documented reference is
  `3beeb1d8a525e58ce6352e05061f242c5580e248`, with its own Runtime
  `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`.

| Reviewed source | Composed commit | Scope |
| --- | --- | --- |
| `9c4949bca1bae188cd84a78614eed8e63c09a299` | `747c40b` | Actual delegation result sink handoff |
| `f81fe3b6985f101f85de5d30866126e68b5e2910` | `08edd66` | Original Agent-stop cancellation operation |
| `c0acaaeec02a4067ce73e9632943d61e1466c0cf` | `8b129d4` | Independent tool-cleanup owner failures |

The producer checkpoint `c0acaae` was first inspected statically. Validation
then moved to the explicitly assigned composed checkpoint above, including its
mechanical optional-query-info fixture adaptations. Earlier independent
approvals of other diagnostics were not substituted for this review. Task088
compaction and primary-Fault work belongs to task092 and is not re-approved here.

## Owner and diagnostic boundary review

1. **Handoff is ownership transfer, not delivery or persistence.**
   `session/collaboration.rs::handoff_delegation_owned` extracts only scalar
   identities/counts while the original owner lock is held. It moves the real
   completion into the existing sink, obtains the actual `Result<(), Fault>`,
   then publishes the unchanged business disposition and emits after unlock.
   Success never asserts a saved parent notification. Explicit sink rejection
   preserves its original close Fault and does not retry or recreate the child.
   The top-level chain is the actual child Session plus registered task
   MessageId. The ended parent Turn/tool is only `origin_*`; actual result Turn
   IDs come from the transferred results. No current Turn/tool/operation is
   fabricated. Result text and configuration are not included.
2. **Agent stop preserves the first actual claim.** Both real callers pass
   their existing operation into `set_agent_enabled`. The claim is derived from
   the original Starting/Running transition, not a new logging flag. Repeated
   stop, a finished Turn and a child with no active Turn add no duplicate claim.
   Optional operation metadata is copied only when present. Agent traversal,
   admission and Session locks are released before emission. Runtime cancel,
   activation expiry, queue ownership and receipts are not replaced by logging.
3. **Cleanup is confirmed only by the original handle return.**
   `finish_tool_cleanup` still claims once and awaits the real registered
   handles. Only the independent returned Faults generate `turn.cleanup_failed`.
   The primary Fault is not overwritten or replayed as another primary ERROR.
   Type/code/diagnostic ID, actual outcome/persistence and original Turn/Load
   context survive. Confirmed cleanup drops its original handles before output;
   an unconfirmed return keeps the original Finalizing barrier and resources.
   No diagnostics state authorizes cleanup, repeats it or releases its owner.
4. **Lean shared schema.** The added records contain existing identity strings,
   scalars and a scalar ID array only. Emission runs after owner publication and
   unlock. No new logger service, public method, persistent business mirror,
   quota, recovery mechanism, retry or owner-retaining diagnostic object was
   added. Fault messages/details, prompts, scripts, responses and output bodies
   are excluded by the existing EventData fault-identity adapter and attribute
   allowlists.

## Independent probes

Three private test cases use actual Host, Runtime and OpenAI-compatible
loopback HTTP. Gated sinks and cleanup results are explicitly named fixtures;
they are not live providers, actual device failures or fabricated owner events.
The handoff/stop probes adapt the committed mature fixtures and add independent
assertions; the cleanup probe registers and transfers its own real cleanup
handle. A forwarding subscriber uses the production JSON pipeline and, at each
reviewed event, checks public Host/admission, Agent and Session reads on a bounded
worker while the emitting callback is still active.

- Handoff: while the sink owns its received payload but has not returned, no
  handoff record exists. After success, the temporary memory child is actually
  closed/reclaimed before notification. A parent whose original Turn has ended
  and Load is unloaded can still receive the valid notice; a separately closing
  parent explicitly rejects it. Both remain a successful sink handoff, without
  a false delivery/persistence assertion. Explicit sink failure instead yields
  exactly one failure summary and retains the close fault. All callback owner
  reads complete. The exact child task/result IDs and old parent origin IDs match.
- Agent stop: the original stop operation yields exactly one claim and a
  Cancelled result. After an explicit Session pause and Agent restart, a new
  accepted queued message survives cancellation of the old finished Turn and a
  later stop. No extra model request or second old-Turn claim occurs. The callback
  can query all owner layers without blocking.
- Cleanup: a gate before `finish` returns leaves Finalizing, no cleanup-error
  record and the original handle alive. The unknown fixture Fault
  `FUTURE_CLEANUP_LEAF` / `62093` retains its original diagnostic ID without its
  private message/details. Confirmed and unconfirmed returns are checked with
  an open pipeline, a genuinely closed pipeline and filtered-out diagnostics.
  The registered handle has no fixture-side strong owner: Weak proves release
  after confirmation and retention after an unconfirmed return. Primary 4204
  remains unchanged, each handle runs once, unconfirmed work has no Turn result,
  and a later cancellation cannot bypass its barrier. Open output has exactly
  the primary ERROR plus the distinct cleanup ERROR per Turn. Closed-output
  drops are observed without changing these business facts.

### Fixture/setup corrections retained, not production red tests

- Unseeded private lock generation was blocked by the already-pinned yanked
  Tokio version. Seeding from the reviewed Cargo.lock allowed offline resolution;
  generic metadata then stopped on an uncached irrelevant-target `bumpalo`.
  Actual builds/tests succeeded with `--locked --offline`. A parsed lock identity
  comparison found no dependency version/source/checksum not already in the
  reviewed lock; the only new package is the private probe package itself.
- The first stop probe incorrectly expected a stopped Agent to accept new
  input. It was corrected to explicitly pause the Session and restart the Agent
  before accepting new input, using the original authorized client for mutation.
- A subsequent stop assertion compared entire InputList values, including the
  legitimately advanced Controller revision. It now compares the actual queue.
- The first forwarding subscriber published its private observation before
  forwarding into the production writer, allowing probe shutdown to race the
  pending output. It now forwards first. No production change was made for any
  of these fixture issues. Original failed output remains in the evidence set.

## Actual commands and results

All final build/test commands used Rust 1.90.0, the normal locked public Runtime,
`--locked --offline`, `RUSTUP_HOME=/workspace/shared/rustup`,
`CARGO_HOME=/workspace/shared/cargo`, `/workspace/shared/cargo/bin` prepended to
PATH, `TMPDIR=/workspace/shared/minicore-host-tmp`, `CARGO_BUILD_JOBS=2`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0`,
and the exclusive `CARGO_TARGET_DIR=/workspace/shared/minicore-host-task093-review/build`.
No target directory was borrowed from another worktree.

| Command (from the fixed worktree unless noted) | Result |
| --- | --- |
| `cargo test -p minicore-host-transport --test diagnostic_handoff --test diagnostic_agent_stop --test diagnostic_cleanup --locked --offline` | 3/3 passed |
| `cargo test -p minicore-host-core --all-targets --locked --offline -- --list` | 337 unique listed core cases |
| `cargo test -p minicore-host-core --all-targets --locked --offline` | 337 passed, 0 failed/ignored |
| `cargo test --manifest-path ../evidence/probes/Cargo.toml --locked --offline --no-run` | All three private probe binaries built |
| `cargo test --manifest-path ../evidence/probes/Cargo.toml --locked --offline --no-fail-fast -- --nocapture` | Final v3 run: 3/3 passed |
| `cargo fmt --all --check` | Passed |
| `cargo clippy -p minicore-host-core -p minicore-host-transport --all-targets --locked --offline -- -D warnings` | Passed |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-core --no-deps --locked --offline` | Passed |
| `./scripts/check-whitespace.sh` and `git diff --check` | Passed |

This reviewer did **not** run a complete new workspace gate, workspace doctests,
Python checks, performance measurements, real external-model execution, hosted CI
or TUI acceptance. The 337 core count is deliberately not a workspace count and
does not reuse the producer's older 317 result. Root must run the final composed
workspace gate and its workspace-wide unique-case listing separately.

## Evidence and integration boundary

Private evidence directory:
`/workspace/shared/minicore-host-task093-review/evidence`.
`SHA256SUMS` covers the raw logs, exact independent probe sources/manifest/lock,
and retained failed setup/fixture output. Manifest SHA-256:
`a843bd63283c0b4c4611a4b1d228fdcc67591aa66b455bfd42e01e46823c7559`.

Key raw log SHA-256 values:

- Focused: `e543b8b5ddd945984672d42683dad19205a6a79bbf61dddac92bf5e1264091cf`
- Core list: `dd4ec8a0fbcbfbcd7a855e5da487485efe863a354ab69f87e7b0c360109664c6`
- Core run: `a30e233250a3b226c8b6aebbb5bda214df787de1adfd9bf17e4b8f7600f4dc96`
- Final independent probes: `3c0f6d05ab3d13e389aafa4df6782140c28018eea9ae7e55af503d3d5950aed1`
- Strict Clippy: `3bcb3d88959abd1a8ed08f67d2d7ad5d4ce272c53ae078393b30dd225e04dd26`
- Strict rustdoc: `33fdf28ea061185e551d42bd7c14ad3fa82aed96d07e3e4fd3a927b0d0d28dcf`

No production code, SPEC, lockfile, TUI, main branch or remote ref was changed by
this reviewer. No push was performed. Resource-preparation/failure-scheduling
and force-claim diagnostics were not yet assigned as a fixed composed revision
at this checkpoint and require a separate explicit follow-up verdict.

## Follow-up A: force, preparation and internal-message diagnostics

### Separately fixed scope and verdict

**APPROVE for these three additional slices** at
`11e5bf5c05750bc4a39fd9e2c295ee8f39d0f246`, tree
`3a8e011c327d6b3283fc6d69e85f1cd5e65794a0`:

- `7bcc10e104b1226eb2561429dc2b538447d7f3da`: actual force claims, replayed
  from source `1fef32ca5d7ae7a670018afa26a22142e1ff5003` onto the earlier
  `8b129d4` checkpoint. These are two mappings of one change, not two patches.
- `55bd353abcb5916a523821d609cd73cd2d0a9f16`: actual resource-preparation
  refusal and its changed scheduling projection.
- `11e5bf5c05750bc4a39fd9e2c295ee8f39d0f246`: failed internal delegated-send
  and notification invocation correlation.

The earlier eight-character checkpoint references and evidence above remain
unchanged. This follow-up does not silently widen the original three-scope
approval. SPEC and Cargo.lock hashes remain identical. No production file was
edited by the reviewer. Requested/effective reviewer configuration and the
unchanged TUI/Runtime distinction remain as stated above.

### Findings and boundary checks

- Both force paths add a record only when their original Starting/Running
  state changes to Cancelling. Session traversal uses the revisions it really
  publishes. `SessionInputs::take` does not publish at that boundary and omits
  both revisions. Neither callee receives an operation, so neither borrows the
  graceful or later force operation from an enclosing span. No cancellation
  token, input disposal, activation lease or existing return path was changed.
  Static inspection and distinct actual-path probes found no duplicate claim.
- Preparation diagnostics occur only after an actual accepted candidate error
  is retained as the owner's block. The stale-config `continue` remains before
  all new record construction. Queued counts and the effective config/Controller
  revision come from the same publication; the allocated but unbound Turn ID is
  not logged. The five-scalar scheduling comparison emits only a changed
  projection. Ordinary resource/config/lifecycle rejection is WARN; unexpected
  and unrecognized failures are ERROR without remapping their Fault identity.
- Internal message wrappers preserve the original permission checks, existing
  one-attempt notification flag and exact `Result`. They emit only after the
  failed invocation has returned and all nested admission/owner guards have
  ended. Original input MessageId and `in_reply_to` are captured before moves or
  notification canonicalization. Source and requested target remain distinct.
  Notifications use the actual registered child chain and only origin metadata
  for the ended parent Turn/tool. Failed calls are WARN/DEBUG, never a second
  primary ERROR. The context holds no body, owner, request object or raw Fault.

### Additional independent tests

Four private cases passed against this exact source checkpoint:

1. Real persistent force upgrade and repetition: callback reads all owner layers
   without blocking, one original Turn is saved Cancelled, only the actual
   published revisions appear, and no unavailable operation is guessed.
2. **Deterministic real Runtime input-fence path:** a fixture PromptProvider
   pauses the actual Runtime between input boundaries. Tracing gates hold the
   accepted force call and its original shutdown worker *outside* owner locks.
   Force admission has already happened; the prompt is released so the real
   `SessionInputs::take` wins the first claim. This changes no owner fields and
   synthesizes no owner event. That callback reenters Host/Agent/Session reads,
   has neither operation nor unpublished revision, then releases the ordinary
   traversal. Exactly one claim and one Cancelled finish result; zero HTTP
   requests occur. This is not an uncontrolled repeated-race experiment.
3. A gated first resource candidate becomes stale after a real Session config
   update. Its returned Fault has no diagnostic record. A second actual failure
   retains the original queued message and records revision 1 once. Both a known
   missing-resource WARN and an unknown `FUTURE_PREPARATION_REFUSAL` / `62094`
   ERROR preserve actual identity, body exclusions and the existing resume
   policy. Refusal and scheduling callbacks reenter owner reads.
4. Failed task send and duplicate/failed notification record exact original
   MessageId, reply link, source and target. A retained original Turn capability
   is also invoked after its parent Turn ends: it still returns
   `HostAccessInvalid`, and the child never accepts that attempted message.
   Logging does not reactivate the capability, widen permissions or emit another
   primary ERROR. Actual callback reads complete without owner-lock retention.

The preparation probe's first compile attempted to compare `BlockReason` directly
with Fault. It was corrected to inspect `BlockReason.fault`; its compile-only log
is retained and is not called a production failure. No production correction was
required. The committed handoff fixture additionally executes its business path
without any installed subscriber before enabling its real JSON pipeline.

### Fresh validation and evidence

The same exclusive environment/target settings described above were used.

- `cargo test -p minicore-host-transport --test diagnostic_force --test diagnostic_preparation --test diagnostic_handoff --test diagnostic_agent_stop --test diagnostic_cleanup --locked --offline`: **5 passed**.
- `cargo test -p minicore-host-core --all-targets --locked --offline -- --list`,
  then the same command without `-- --list`: **337 listed unique core cases,
  337 passed, 0 failed/ignored**. This remains a scoped core count, not workspace.
- `cargo test --manifest-path ../evidence/followup-probes/Cargo.toml --locked --offline --no-fail-fast -- --nocapture`: **4 passed** in the final run.
- `cargo fmt --all --check`, strict core/transport all-target Clippy, strict
  core rustdoc and `./scripts/check-whitespace.sh`: **passed**, using the exact
  same commands as the preceding scope.

Raw follow-up logs are `source11-*.log`; exact private sources/manifest/lock are
in `evidence/followup-probes/`. The separate `FOLLOWUP11_SHA256SUMS` manifest has
SHA-256 `f6adc629e5a3e98f3e71535490e668d4d948c185596cf10703ea5e6a2174c885`.
It does not replace the immutable earlier evidence manifest.

- Focused log: `94e5f7aaf696b412bd909350635a03d003f62e42ed1e69c6a4322ad30a312d55`
- Core log: `da6b84e5f8fc3f6fba9b7aafc47d46a716a3fcd70f2a41b823bf1bea15097024`
- Final independent probes: `c4269be36230f041041f7efff45385a91881ef7694e89f34af179f228f81a591`
- Strict Clippy: `e30789724e469e07ae23b438537bf20d3de3b0928231c61675d7aa588a164a67`
- Strict rustdoc: `acaad188f3067848af4ba1ecdeb726d75c82bb089f560093c4558524ec2fdb1f`

No full workspace gate, performance/live-provider/TUI acceptance or publication
is claimed by this follow-up. The later composed compaction baseline and the
subsequent thin failure/save-resume scheduling patch are separate review scopes.
