# MiniCore Host

The first v4.93 release is **not complete or release-ready**. The current local integration code checkpoint is `d107462e1e67b2270c268a0b4f51bb4ba01bde00`, compared with local foundation `797622fa9d663db01fad0d8b800a51a181ddc150`. It passes 477 unique tests and 3 doctests. This later code is not yet published or declared independently accepted. [Task 029](tasks/029-owner-resource-integration/TASK.md) records the scope, checks, findings and remaining work.

The earlier foundation is available on `dev` at [82583b31ac9091164da5f6dcfee6bb23e05137e5](https://github.com/zqcli/minicore-host/commit/82583b31ac9091164da5f6dcfee6bb23e05137e5), compared with unchanged remote main `b341b163e14543bfbe7f8d392ea243b4a123cdf7`. Its exact-head Host CI passed, with 340 tests and 3 doctests. [Task 028](tasks/028-host-publication/TASK.md), the [101-commit mapping](docs/host-publication.json), and the [foundation review handoff](docs/review-handoff.md) remain fixed historical evidence. Later documentation-only remote updates do not include this local integration.

## Fixed dependencies

- SPEC v4.93 remains complete and unchanged, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- All six Runtime dependencies and the sole lockfile select actual published Git revision `69faf9196991524b5549499d6208a1eaf567bf56`, exactly `0.10.0`, on `feat/host-history-handoff`. There is no path patch, vendor copy or bootstrap source. It adds authoritative retry-progress observation to the earlier history/partial/quota work. [Publication mapping](docs/runtime-retry-publication.json) and [CI run 42624211012](https://github.com/zqcli/minicore-runtime/actions/runs/42624211012) verify the exact tree and all seven successful jobs. Earlier Runtime `59ee8ce` remains the foundation pin, not this current pin.
- Runtime upstream dev reference is unchanged `3fd5ecda1fb7c717ab0a39218d90268b4d97909c`.
- Agent reference dev `f36a87366d8e2fe0a1c8216463887902e471c739` and TUI reference dev `3beeb1d8a525e58ce6352e05061f242c5580e248` remain unchanged. Both actually lock Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`; no joint integration or TUI adaptation is claimed.

## Implemented scope

- Strict protocol contracts, direct-owner IDs, complete 52-name catalog, validated receipts/Faults and lossless unknown Fault leaves. The catalog is not 52 completed handlers; see [method coverage](docs/method-coverage.md).
- Linux durable object/native-history storage, inode ownership, exact accepted versus persisted ranges, corruption rejection and original-save continuation. Memory objects create no files.
- Production OpenAI-compatible HTTP/SSE model adapter, conservative retry classification and Retry-After, authoritative partial data, actual model constraints and safe credential handling.
- Host/Agent/Session owners, independent input queues, real Runtime, same-Turn continuation, frozen execution binding, canonical controls, accepted task lifetime, save barriers, monotonic shutdown and memory reclamation.
- Scoped observation with fixed ranges, UTF-8 catch-up, cold full history including authoritative partial records, real tool invocation reads and retry state. Typed clients, real stdio and authenticated WSS now share 31 actual routes, including subscriptions and standard interaction answers. Original interaction settlement follows actual answer, cancel or deadline facts. This is an embedding library, not yet a production process.
- Standard six-action `agent` tool with owner-bound capabilities, early registered independent delegation results, shared input ranges, notifications through the same queues, wait and scope-close cleanup. Metadata updates support actual Agent name/Session title with durable publication.
- Real configured workspace file/search/Git backends and edit snapshots. Canonical workspace routes and ordinary write tools are still missing.
- Production resource assembly from a single HostConfig: actual provider clients, static tool declarations, workspace binding, frozen Role/framework/context and versioned Host-owned reload publication. Resource queries exist in core; canonical routes and reload receipts remain incomplete. Credentials are private inputs, not returned configuration.
- Per-request legacy tool-history projection is wired into the production BoundPrompt. Complete exchanges retain their content and associations; absent old results become historical facts only in the request. Native history/JSONL are not rewritten and old tools are not re-executed.

## Validation and review

At `d107462`, `./scripts/check.sh` passes committed/staged/worktree whitespace, rustfmt, all-target Clippy with warnings denied, all workspace tests, 3 doctests and strict rustdoc. `cargo test --workspace --all-targets --locked -- --list` confirms **477 unique target/test-name cases**, excluding repeated storage subprocess output. `cargo metadata --locked --offline` confirms actual Runtime Git `69faf919`. Rust is pinned to 1.90.0.

These checks use real owners, AgentLoop, temporary files, actual pipes and loopback TCP/HTTP/SSE/TLS with explicitly named scripted models or local provider fixtures. They are not live remote-model, production-process, TUI or capacity measurements.

Independent reviews found real defects after earlier passing gates. Authoritative partial history reads and subscription teardown were fixed and re-probed. Two delegation P1 defects were corrected in source `aa76ff9`: explicit load exit now transfers a pending result before reclamation, and accepted same-relation appended input can establish the actual shared result range. Independent review passed 450 tests/3 doctests plus seven additional real probes and closed those two findings. The actual deadline interaction label was fixed separately; its independent follow-up is pending at this checkpoint. A reproduced pipe BrokenPipe came from a test closing its response reader while its real unsubscribe response was in flight; deterministic half-close tests now preserve both valid orderings without changing production error handling.

The resources and prompt projection have separate independent production-chain review in progress. Passing their individual gates is not declared complete product review. Integration first found a legacy HostSettings test fixture that no longer matched canonical HostConfig; only the fixture was corrected, with the failed compile and formatting run retained in task evidence.

## Remaining first-version work

1. Fix explicit Session resume/config-update clearing of recoverable resource/configuration startup faults; never clear history or saving failures indiscriminately. This normal resource-repair case is under implementation.
2. Complete canonical resource queries/reload receipts, metadata/input/start/default command routes, persistent deletion and the rest of the 52-method surface.
3. Implement manual/automatic compaction, real context accounting and durable compaction ownership. Production currently rejects enabled compaction explicitly; because the default is enabled, production test configurations must explicitly disable it. This is an unfinished feature, not a silently supported default.
4. Supply ordinary read/edit/bash tools, canonical workspace routes, actual write capture and retention, and full model input source/association context. Stored message provenance is preserved, but Runtime UserHistory alone does not currently expose it to the production prompt.
5. Assemble the real Host executable/configuration startup and TLS/logging, then run full process-level acceptance, authorized live-provider checks, performance measurements and the requested external review. TUI stays untouched this round.

## Scope constraints

Necessary save failures cannot become successful stopped shutdown. Repeated shutdown returns facts, not an implicit retry. No new recovery API or witness framework is added for unpublished creation save failure. Dropping the final root Host after an already-failed save without controlled shutdown is uncontrolled destruction, not successful retirement; no permanent self-cycle is introduced. Workspace Git preflight assumes stable local configuration/attributes, not atomic protection against arbitrary external mutation.

Development stays on isolated topics and integrates into `dev`. Publication must preserve existing remote history and use the owner's current explicit authorization; no force update or main merge occurs. Independent review and required checks remain prerequisites for main. Workers were requested with `gpt-6-astra` and `max`; no separate runtime configuration attestation is available.
