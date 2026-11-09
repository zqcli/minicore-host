# Task 042: Bind production local tools to their real owners

## Scope and fixed inputs

- Isolated branch `feat/production-local-tools`, based on `4572488edf053b4f9d7e5f0c62225327506b0d97`. The frozen publication checkout and main are not modified; no push is performed.
- SPEC v4.93 remains byte-identical, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- Runtime remains the actual pinned Git `69faf9196991524b5549499d6208a1eaf567bf56`, exactly 0.10.0. TUI is unchanged; no live provider or TUI acceptance is claimed.
- Prerequisites are preserved as separate commits: `17661c4` (source `f1a33d5`, static local-tool dependency bootstrap), `34ec623` (source `7f7c280`, Turn-owned output lease and registry adapter), and `79d13fa` (source `e8cceab`, coordinator-owned core-to-workspace dependency). The sole cherry-pick conflict kept both the existing compaction exports and new output lease export.
- Requested worker configuration: Astra/max. The separate acceptance worker was explicitly requested with `gpt-6-astra`, `max`, and no history fork. This records the request, not unavailable runtime attestation.

## Ownership and integration

`HostResourceBuilder::builtins` statically registers agent/read/write/edit/bash. Host configuration enables available declarations; Session selection constructs one actual ToolSet used for both model declarations and execution. Local tools require an explicit configured Workspace. Production calls `BoundLocalTools::production`, which constructs the real Session adapter; it never enables writes through `NoopLocalToolSink`. The standalone/custom sink entry point remains explicit.

The existing prepared Turn binding admits execution and registers the existing real `LocalTools` collector before activation. The actual Runtime invocation obtains a Session-minted output lease before its worker starts. No identity comes from model arguments or ToolStarted events. Cancellation can expire ordinary execution authority while the original cleanup-scoped lease still admits actual worker tails for the same Host incarnation, Session, Turn, Load, call, and response index. The actual per-call completion callback closes that lease after file I/O or process/group/pipe cleanup has settled. Native result handoff and saving remain immediate; public Finished state and its one completion event wait for both that result and confirmed actual cleanup. The existing once-per-public-Turn collector still joins workers and closes any remaining leases only on confirmed cleanup. Normal completed calls and argument rejection do not wait for Turn finalization. No process scheduler or process-group cleanup implementation is added or replaced.

One builder owns a small weak map of canonical Workspace roots. Binding performs filesystem work before entering the map mutex; concurrent candidates then reuse one actual Workspace object and its existing mutation guard. Builder clones and old/new published resource snapshots share this map. Each Turn retains its resolved root and immutable configuration through its original prompt/tool resources. The map does not retain unused Workspace objects. It is not a process-global lock service; separate Host builders, arbitrary shell actions, and outside processes are not coordinated by it.

Actual write/edit callbacks append the observed path, before/after bytes or unknown state, outcome, and owner-supplied ToolCallRef to the existing Session-owned EditJournal. Repeated edits remain separate ordered records. Git status is never used to attribute an edit, and bash file changes remain unknown. The narrow `Host::session_changes` and `Host::session_change_record` hooks follow Host → Agent → Session authorization and return retained observations. They do not register `workspace.*` or `changes.*` transport routes. The existing workspace diff helper can consume a cloned record outside owner locks.

## Text and retention boundaries

The adapter retains only invocation leases, raw received-byte counts, and incomplete UTF-8 code points. Each stream validates actual producer offsets. Complete UTF-8 prefixes are emitted unchanged, with existing UTF-8 byte positions; split multibyte characters wait for the next real chunk. Invalid bytes or a final incomplete code point stop that channel's text projection without replacement characters, escaping, renumbering, or later suffix splicing. Already emitted text remains unchanged. A text read crossing that real prefix returns `SessionOutputRangeInvalid`, explaining the binary boundary; reads wholly inside the actual prefix remain valid. A genuinely empty observed stdout/stderr is registered as empty rather than missing.

The successful native bash CommandResult remains byte-exact, using its existing `encoding: bytes` representation when needed. The standard ToolResult/history path is authoritative; the command callback only checks/finalizes streams and does not invent a second lifecycle/result store. Cancellation/deadline may produce a native cancelled/timed-out result rather than the callback's full command detail; no independent permanent command archive is promised.

Edit snapshots and process-text observations are retained by the Session during this Host incarnation, including ordinary unload. Actual source reclamation clears them and weak leases cannot keep a memory Session alive. They are not restored after Host restart. A reopened persistent Session reports Session-wide edit coverage unavailable (`SessionOutputNotFound`) instead of pretending that an empty list is complete; newly retained current-incarnation Turn-scoped edits remain queryable. Old specific edit/output sources fail as not retained. Existing durable native ToolResults remain available through the ordinary history reader after reopen. No permanent output library, edit database, quota/eviction policy, source reconstruction, or extra recovery API is introduced.

## Scoped minicore-agent reference comparison

Reference only: `/workspace/shared/minicore-agent-reference`, exact revision `f36a87366d8e2fe0a1c8216463887902e471c739`.

- `src/tools/mod.rs`: reuse the design of selecting actual implementations and schemas together. Host uses its existing static registry and PreparedTurnBinding rather than Agent's PresentationTool wrapper and mutable request context.
- `src/tools/observe.rs`: retain narrow actual-source recording, but bind once from Session-minted Host/Turn/Load identity instead of consulting the reference's current request key or reconstructing state from events.
- `src/tools/file_change.rs`: retain observation of the actual source/commit/after boundary and ownership of already-started writes. Host's already-reviewed local-tools implementation and Workspace mutation guard remain the implementation; this stage only connects the existing EditJournal to its Session.
- `src/tools/command.rs` and `src/tools/command/*`: keep the mature actual worker/group/pipe ownership already adapted by the local-tools module. This stage adds no replacement command executor.
- `src/tool_data.rs`, `src/tool_data/data.rs`, `src/tool_data/record.rs`, and `src/changes.rs`: do not import their budgets, eviction windows, durable auxiliary blobs, request/loop identities, or separate tool recording lifecycle. SPEC's current owner retention, native result history, and UTF-8 text protocol govern this Host implementation instead. This is a source/design comparison, not a compatibility baseline or a numerical quota target.

## Validation

Initial code/test checkpoint: `6b4bef5`. The independent review then found P1 described below; this checkpoint is not the corrected acceptance checkpoint.

Implementation commits after the prerequisite commits:

- `6fcc85a`: `feat(core): retain actual tool edits under the Session owner`.
- `70f731d`: `feat(tools): record local observations through original Turn leases`.
- `1045aa4`: `fix(core): distinguish fresh edit coverage from reopened Sessions`.
- `10fd934`: `feat(resources): bind actual local tools and shared workspaces`.
- `1bb488b`: `fix(core): keep binary text faults independent of result retention`.
- `6b4bef5`: `test(resources): verify actual local tools across owner boundaries`.

The initial complete workspace gate passed before that independent P1 finding:

- `./scripts/check.sh`: whitespace, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --all-targets --locked`, `cargo test --workspace --doc --locked`, and `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` all passed.
- `cargo test --workspace --all-targets --locked -- --list`: **659 unique target/test-name cases** across 66 nonempty test binaries. Nested storage subprocess probe output is not counted again.
- Documentation tests: **4 passed** (diagnostics, provider, storage, and transport: one each).
- `cargo metadata --offline --locked --format-version 1`: exactly one Runtime package, version 0.10.0, Git revision `69faf9196991524b5549499d6208a1eaf567bf56`.
- `git diff --check` and the unchanged SPEC SHA-256 check passed.

Initial gate evidence: `/workspace/shared/production-local-tools-final-gate.log`, `/workspace/shared/production-local-tools-final-list.log`, and `/workspace/shared/production-local-tools-final-metadata.json`.

The ten new `host-resources/tests/task042.rs` cases use the actual production Host, resource builder, HTTP/SSE provider, Runtime, filesystem and bash process. Only loopback model replies are scripted. They verify actual read/write/edit/bash results and declaration/schema equality; each native result reaching the next HTTP model request; an unselected write causing no side effect; repeated provider IDs across Turns; live multibyte output and complete source identities; invalid bytes and incomplete EOF with unchanged readable prefixes and byte-exact native results; frozen tool/workspace/model/Role selection across reload/update; cancellation and Runtime tool deadline with actual child disappearance before Turn finished; reclamation despite retained observers; and persistent native history after reopen while observation retention remains explicitly limited.

Two private resource cases verify canonical alias reuse and weak-cache release, plus two concurrent real production Sessions on old/new published resources sharing one actual mutation guard. The latter holds that guard as its gate, checks actual shared binding retention, releases both real edit tools, and verifies both disjoint changes plus the complete serialized before/after chain. It also confirms old/new provider bindings remain separate across reload. Three tools unit cases probe split multibyte decoding, binary/EOF boundaries, and reordered/missing raw byte offsets. The earlier gated actual-process cleanup and registry integration cases still pass unchanged.

The first acceptance run caught a real bug: fresh persistent Session creation also uses `from_record`, so treating every such construction as a reopen incorrectly hid its new journal. The explicit internal fresh/reopened distinction fixed that issue; the original assertion passed without weakening it. An initial live-stream test assertion incorrectly expected a still-pending code point to be a binary error; the corrected test asserts the actually available UTF-8 prefix until EOF. The concurrency test's initial compile errors were fixture payload/category type mistakes and were corrected before execution. No production fallback, sleeps-as-completion proof, or retry was added to satisfy a test.

Unix-only process cases require bash and FIFO support. Linux-only cancellation/deadline checks prove the directly owned child has disappeared via `/proc`; they do not claim validation of escaped descendants. Existing local-tools process-group cleanup remains unchanged and independently covered by its own cases. This stage does not validate a live external provider, a TUI, or the later canonical workspace/changes transport routes.

A separate workspace/changes route stage may map the retained-edit query errors to its coordinator-approved specific workspace/change Fault leaves. This isolated stage intentionally uses the existing registry; that later mapping must preserve the tested fresh/reopened and current-Turn retention behavior and update exact-kind assertions with the transport integration gate.

Completed preliminary checks:

- Existing production resource/tool test suites passed after initial wiring; log `/workspace/shared/production-local-tools-initial.log`.
- Strict focused all-target Clippy passed; log `/workspace/shared/production-local-tools-focused-clippy.log`.
- Original real-tool cleanup regression target passed; log `/workspace/shared/production-local-tools-cleanup-regression.log`.
- Incremental UTF-8, binary EOF, raw offset checks and weak canonical Workspace mutation-binding unit cases passed; log `/workspace/shared/production-local-tools-focused-lib.log`.

Environment: Rustup/Cargo homes under `/workspace/shared`, target `/workspace/shared/minicore-host-production-tools-target`, two Cargo jobs, debug information and incremental compilation disabled, project TMPDIR. Lockfile is used as committed, never regenerated.

## P1 review correction: actual per-call completion

The independent review of `6b4bef5` reproduced premature public ToolCall completion: Runtime handed over its cancelled/timed-out native result while the real local worker still owned process/pipe cleanup. The initial event carried stdout end 5 and no stderr; the original lease subsequently accepted actual tails through stdout 14 and stderr 10. Native result uniqueness and the final Turn cleanup were already correct. The earlier tests checked Turn completion, not the per-call completion boundary.

The correction is deliberately narrow:

- `4f0988b` (`fix(tools): publish completion after actual call cleanup`) connects the existing local worker's settled outcome to `LocalToolSink::complete`. The production adapter confirms its original Session-minted lease only when actual cleanup is confirmed. The existing shared admission flag gates both public Finished state and its completion event; no extra lifecycle enum, scheduler, result store, or process implementation is introduced.
- Native ToolResult handoff/history saving remains immediate and authoritative. If it precedes cleanup, the result is queryable while the tool state remains unfinished. If cleanup precedes the native result, native handoff publishes completion immediately. The synchronous callback advances the existing runner revision; finished state and final endpoints are published exactly once.
- Pre-worker argument/context rejection reports actual absence of a worker immediately. `cleanup_confirmed=false` never closes the lease or pretends the tool is finished. The existing Turn cleanup barrier remains a fallback for other trusted tool bundles that do not supply earlier per-call completion, and still performs actual joins once.
- Mature command execution, process-group supervision, pipe draining, and Scope scheduling are unchanged. The callback is after their real work, not after a Runtime event or future cancellation.
- `e330db7` (`test(tools): gate finished state on real worker cleanup`) adds permanent real cancellation and timeout tail regressions, checks Finished state as well as completion, verifies normal success and invalid arguments finish while the next actual HTTP model request remains gated, and strengthens existing generic cleanup tests to show native history is immediately readable while cleanup is still blocked or unconfirmed. Standalone rejection also proves no worker was started and no Turn-wide finish wait is required.

Original reviewer artifacts remain unchanged in `/workspace/shared/minicore-host-production-tools-review-artifacts/`: `review_probes.rs`, `review_probes_cleanup.rs`, `independent-probes.log`, `cancellation-repeat.log`, and `cleanup-probes.log`. The two source files are copied byte-for-byte only for their test run, then removed from the worktree. The permanent regressions retain the same real FIFO/child gating, add public state assertions, and do not substitute a fake sink. The timeout test deliberately stalls its executor past the configured deadline to reproduce a buffered real tail; it still proves completion with actual pipe contents and child disappearance, not elapsed time.

Corrected fixed code/test checkpoint: `e330db7c2afb815cbceb04b25875f06bb3ca0e2b`. The corrected complete workspace gate passed:

- `./scripts/check.sh`: whitespace, formatting, strict all-target Clippy, all workspace targets, documentation tests, and rustdoc with warnings denied all passed. Log: `/workspace/shared/production-local-tools-p1-final-gate.log`.
- `cargo test --workspace --all-targets --locked -- --list`: **664 unique target/test-name cases** across 66 nonempty test binaries. The five additional cases are four production resource completion cases and one standalone pre-worker rejection case; existing core cleanup cases were strengthened. Nested subprocess probes are not extra cases. Log: `/workspace/shared/production-local-tools-p1-final-list.log`.
- Documentation tests: **4 passed**, diagnostics/provider/storage/transport one each.
- `cargo metadata --offline --locked --format-version 1`: one Runtime 0.10.0 package at unchanged Git revision `69faf9196991524b5549499d6208a1eaf567bf56`. Evidence: `/workspace/shared/production-local-tools-p1-final-metadata.json`. `sha256sum SPEC.md` and `git diff --check` passed.
- Focused final command: `cargo test -p minicore-host-resources --test task042 -p minicore-host-core --test task033_tool_cleanup -p minicore-host-local-tools --test files --locked`: **35 passed** (14 + 9 + 12). Log: `/workspace/shared/production-local-tools-p1-permanent-tests-final.log`.
- Both reviewer files were copied unchanged, verified with `cmp`, and run on the fixed checkpoint using `cargo test -p minicore-host-resources --test review_probes --test review_probes_cleanup --locked -- --nocapture`: **4 passed**. Each reports final stdout 14, stderr 10, child absent, no post-completion output, and a unique native result. These temporary targets are not part of the 664-case inventory and were removed before the full gate. Log: `/workspace/shared/production-local-tools-p1-fixed-probes.log`.

All production/test changes are fixed in the two granular commits above. This is the worker's validation, not a claim of completed independent review or publication; the coordinator will re-review this checkpoint and gate the combined integration. No push or merge was performed.
