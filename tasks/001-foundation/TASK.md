# Build Host Foundation

Status: the real Runtime foundation, scoped persistent-owner milestone and aggregate validation are complete. The full v4.93 release is not complete; remaining lifecycle/service/transport work is listed in PROJECT.md.

## Goal

Implement typed ownership contracts, durable object/history storage, connection operation rights, Session double queues, and a real pinned Runtime vertical slice. Preserve the full v4.93 scope and record subsequent work explicitly.

## Source and environment checkpoint

- Full spec copied from the user's exported v4.93 source; SHA-256 verified as `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- `git clone https://github.com/zqcli/minicore-host.git` completed with the expected empty-repository warning.
- Linux x86_64 with Git/C compiler/curl available; Rust/Cargo initially absent.
- Official Rust installed from `https://sh.rustup.rs` into workspace-local Rust/Cargo homes. The toolchain is pinned to Rust/Cargo 1.90.0 with rustfmt and Clippy.
- Initial foundation work made no remote writes, TUI changes or live external provider calls. The later approved Runtime publication is recorded below; Host remains unpublished. Real Runtime and loopback HTTP integration results are recorded below.

## Acceptance priorities

1. Strict IDs, text-only messages, typed receipts and direct ownership.
2. Real Session acceptance; independent next-step/next-turn FIFO; no host-wide execution lock.
3. One active Turn; normal finish races with steer at one owner boundary.
4. Complete Runtime records handed off at response/tool-batch boundaries; persisted only after required sync.
5. Cancellation joins and receives authoritative partial output; no reconstruction from observer events.
6. Stable persistent identities, fresh incarnation, exclusive root, strict corruption detection; no execution recovery.
7. First valid connection controls; read-only connections never silently upgrade; stale callers cannot accept new writes.

## Remaining full-version scope

All unimplemented methods and lifecycle branches, DSH delegation, complete provider/tools, stdio/WSS, event/read synchronization, independent review, and real TUI integration. These are not asserted complete by foundation tests.

## Integrated checkpoints

- `98621c9`: complete specification, engineering instructions and initial project record. This remains `main`.
- `86dc6cd`, `6a725aa`, `8c80b4e`, `6ff9607`, `d43ef8a`: strict protocol foundations, input/creation receipts, controls/method catalog and JSON-RPC contract documentation.
- `263860f`, `13c04b3`, `ac347b7`: production OpenAI Chat Completions adapter, real TCP/SSE Runtime integration tests and handoff documentation.
- `7164725`, `2c3a3aa`: real durable storage and native-history/write-failure hardening.
- `75f6dd6`: direct memory owners, connection operation rights, real Runtime turn scheduling and seven reviewed owner regressions.
- `66548fc`: real file-save boundaries, authoritative partial/combined-failure integration and two-turn Host-to-HTTP integration.

All are local reviewable commits integrated onto `dev`. No remote push or main merge has occurred. Workers were requested with `gpt-6-astra` and `max`; no separate actual model/effort attestation is available.

## Validation checkpoint on 2026-11-09

Environment:

```sh
export RUSTUP_HOME=/workspace/shared/rustup
export CARGO_HOME=/workspace/shared/cargo
export PATH=/workspace/shared/cargo/bin:$PATH
```

Commands actually run:

- `cargo check --workspace --offline`: passed after integrating the storage manifest change and refreshing the root lock.
- `cargo test --workspace --all-targets --locked`: 125 tests passed, 0 failed. Protocol 47; provider 27; storage 31; core 20. Subprocess probes re-run selected storage tests; these are not counted again.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed at the preceding core integration checkpoint. It will be re-run after final storage hardening.
- Individual component formatting, documentation and doctests passed as recorded in their task files. The aggregate `scripts/check.sh` gate is still pending after the final changes.
- Runtime's separate full engineering gate at `49a6e8b06512616a2fb9c6104c07536b8396c4ca`: 307 tests and 2 doctests passed, plus format, Clippy/rustdoc warnings-as-errors, documentation, architecture and acceptance gates. This result does not include the newly requested Runtime cleanup yet.
- `sha256sum SPEC.md`: unchanged full baseline hash.
- Remote read-only revision checks still identify Runtime `3fd5ecda1fb7c717ab0a39218d90268b4d97909c`, Agent `f36a87366d8e2fe0a1c8216463887902e471c739`, and TUI `3beeb1d8a525e58ce6352e05061f242c5580e248`; TUI remains clean. Agent/TUI lock the older Runtime `b276d9c0a1f3a3c06e32029f831ccd100c9699c7`.

The core integration runs actual Host/Agent/SessionController/AgentLoop code. Scripted models, gated stores and loopback server responses are explicitly test fixtures. Actual tempfile JSONL and production reqwest/SSE transport are exercised; no live provider or TUI integration is claimed.

## Blockers at the initial foundation checkpoint

1. The required Runtime changes remain local and unpublished. An ignored Cargo patch selects that exact worktree; the current root lock is local-validation state, not a clean-clone release claim. Do not add a bundle/bootstrap or copied Runtime layer. After explicit publication approval, pin the verified remote SHA and regenerate the lock without the patch.
2. Integrate final storage review fixes and re-run the aggregate gate at a stable Runtime cleanup commit.
3. Continue persistent entity composition in its own topic worktree, then the remaining full-version lifecycle/API/transport/tool stages. See `PROJECT.md` for the complete open scope.
4. The separate external review and required integration gates must pass before any main merge. Runtime and Host pushes both still need explicit owner approval.

## Final foundation and persistent-initialization gate

The following small commits followed the first 125-test checkpoint:

- `dcc1fc6`: durability retry, I/O identity checks, terminal partial, exact numeric decoding and incremental validation overlay.
- `9291f0d`: read each Agent's own record without requiring its default Session to be healthy.
- `f70ae2e`: multi-thread owner regressions, condition-based waits and the final-claim-wins steer ordering.
- `7308ae9`: complete storage schema/limits documentation.
- `3945488`: initial persistent direct-owner startup, identity and lazy history/Runtime integration. Persistent mutations are still in progress separately.

At Host `3945488` with Runtime `4f976b41f2381e2b09f3383aa66318b7aee2a8b3`, `./scripts/check.sh` passed:

- Formatting and Clippy for all workspace targets with warnings denied.
- 144 unit/integration tests: core 26, protocol 47, provider 27, storage 44; 0 failures.
- 2 compiling doctests, rustdoc with warnings denied and `git diff --check`.
- The 16-case owner suite separately passed 25 repeated runs. Repetition increases scheduling exposure, not distinct test coverage or a concurrency limit.

Validation output is retained in the engineering workspace as `minicore-host-validation/persistent-initialization-3945488.log`; exact command is `./scripts/check.sh` using the environment above. Earlier checkpoint logs are retained separately, not silently overwritten with newer results.

Runtime cleanup removed superseded test scans/constant assertions and incorrect old handoff guidance; no production source or dependency was removed because the inspected paths remain used. Its final full gate passed 307 tests, 2 doctests, 70 acceptance items and the required architecture/documentation/lint gates. A separate read-only review re-ran 307 tests and 51 focused tests at that exact SHA and found no new blocking issue. This is not the external Host review required before main merge.

The owner approved publishing only Runtime `feat/host-history-handoff` at that exact SHA. The one push attempt returned exit 128 because this environment could not obtain a GitHub username/credential. Subsequent remote reads show the target ref and commit absent; workflow runs for that SHA are empty. No other ref was modified, no force push was attempted, and Host publication remains unapproved. The GitHub connector's create-commit API omits author/committer/time fields and cannot preserve these original commit IDs, so it is not used to manufacture substitute commits.

## Published Runtime pin and post-lifecycle gate

The owner subsequently approved recreating the Runtime commits through the connected GitHub interface, including new SHA identities and its default authorship. Publication is complete on `zqcli/minicore-runtime:feat/host-history-handoff`; no other ref changed. Original-to-published mapping:

| Original local commit | Published commit |
| --- | --- |
| `054a53ee5b293ff23dfd311ec66da4b996cd79cf` | `3f5ee63e97432eeee6059f3c40cb47203ed05e0a` |
| `f817df7da13aa985eb80bad9fef17c50af9b2e29` | `a34a52e293752c9ddb285416ca697b9496f7655f` |
| `5e61c22da4b88891249902ed88184b1eca334504` | `12dc7deb48ded60bec91eded0fb70421ae289e24` |
| `49a6e8b06512616a2fb9c6104c07536b8396c4ca` | `b021e10b5de08ffbf0da7bc0797b8234d50fd045` |
| `af1aa6c96cd13686c345eb30a84cfe645e84bc5e` | `ef8679aabe4f71dba112d606a29aaa86b84093a0` |
| `4f976b41f2381e2b09f3383aa66318b7aee2a8b3` | `18a6ac9b82d003330cd870275efca793cdcc9a96` |

All 55 uploaded blobs, six original trees, exact message bytes and single-parent ordering were verified. Every published author and committer is `zqcli`, using its GitHub noreply address. There are no `Co-authored-by` trailers. Final tree: `46a3dc03a8ec0d89fc18ac39e9924eb1799ec45b`. [Push CI run 42598793185](https://github.com/zqcli/minicore-runtime/actions/runs/42598793185) completed with success for all seven checks, including Linux/macOS/Windows testing and four package targets. These facts supersede the earlier publication blocker, not its historical record.

Host manifests and lock now select published `18a6ac9b82d003330cd870275efca793cdcc9a96`. The ignored active Cargo patch was moved out of the checkout, and the unused vendor symlink was removed. No credential was received, configured or stored to publish through the already-connected interface.

On the source checkpoint following `30c588c`, the following commands actually passed with no local Runtime patch:

- `cargo check --workspace`: refreshed only the Runtime package source in the lock.
- `cargo metadata --locked --format-version 1`: Runtime source is `git+https://github.com/zqcli/minicore-runtime?rev=18a6ac9b82d003330cd870275efca793cdcc9a96#18a6ac9b82d003330cd870275efca793cdcc9a96`; manifest is in Cargo's Git checkout, not the local handoff worktree.
- `./scripts/check.sh`: formatting, Clippy warnings denied, 178 unit/integration tests (core 57, protocol 47, provider 27, storage 47), 2 doctests, rustdoc warnings denied and diff check all passed.

The shell additionally sets `TMPDIR=/workspace/shared/minicore-host-tmp` after the shared `/tmp` filesystem became crowded by independent review build targets. Reviewers removed only their own disposable build artifacts; all reports and source fixtures remain. Log: `minicore-host-validation/published-runtime-pin.log`; source metadata: `minicore-host-validation/metadata-published-runtime.json`.

Two independently reproduced follow-up defects are now fixed in topic worktrees and pending main-worktree integration: metadata read/atomic-rename coordination, and closing an unrelated non-default Session while an Agent metadata candidate has failed. Their final gate and independent counterexample results will be recorded separately. Full version completion, Host publication and external review remain pending.

## Metadata concurrency and close-isolation checkpoint

The final source fixes were integrated as separate commits:

- `f1ec37e8bc04da6f11a2c834173737fd74337c99`: pin the verified published Runtime revision and remove the local dependency prerequisite.
- `a088b00591820a07503e767b8d7c0ba27894ee81`: coordinate only matching metadata-file snapshots and atomic publication, preserving unrelated file progress and unchanged owner/path/link safety checks. The index is held only to acquire/retire file gates, never for I/O or waiting on those gates. Parent/default typed validation does not hold a child file gate.
- `d73b161e8d84d77a19aecd92125d11063192be05`: closing a Session depends only on current or pending default references to that Session. An unrelated failed Agent metadata candidate remains retained but does not block healthy nondefault closure.

At exact Host `d73b161e8d84d77a19aecd92125d11063192be05`, `./scripts/check.sh` passed 190 unit/integration tests (core 60, protocol 47, provider 27, storage 56), 2 doctests, formatting, Clippy warnings denied, rustdoc warnings denied and diff check. Log: `minicore-host-validation/metadata-close-integrated.log`. The storage contributor also repeated the reproduced parent/child metadata stress test ten times: 10,240 saves plus concurrent reads/reopens passed. This is a regression stress check, not a performance or capacity claim.

Clean-source verification used these actual commands:

```sh
git clone --no-local --no-hardlinks --single-branch --branch dev /workspace/shared/minicore-host /workspace/shared/minicore-host-clean-check-d73b161
cd /workspace/shared/minicore-host-clean-check-d73b161
git rev-parse HEAD
test ! -e .cargo/config.toml
test ! -e vendor/minicore-runtime
cargo metadata --locked --format-version 1
./scripts/check.sh
git status --short
git diff --exit-code
```

The clean clone had an empty build target and no local Runtime patch. It used the same workspace Cargo download cache; the verified dependency source remained GitHub revision `18a6ac9b82d003330cd870275efca793cdcc9a96`. The entire 190-test/2-doctest gate passed again and the checkout remained clean. Logs: `minicore-host-validation/clean-source-gate-d73b161.log` and `clean-source-metadata-d73b161.json`. This checks a fresh local clone of unpublished Host commits, not a GitHub Host clone or remotely executed Host CI. The spec hash remains unchanged. TUI/live external provider validation, full v4.93 completion, Host publication and external review remain pending.

## Final foundation handoff

Final code/test checkpoint: `0b9b390750233f7b11ad006563da697014f162e2` on `dev`. Subsequent handoff edits are documentation only. Following `d73b161`, the integration retained these reviewable changes:

- `d2c41ca`: explicit concurrent parent/default-child metadata regression, core README and the memory default precheck outside connection acceptance. The Agent's final closing-set publication check remains unchanged.
- `603405a`, `e2f9ad7`: Linux CI and its exact local verification/provenance. No hosted Host workflow was run.
- `8244ec1`: remove the obsolete member provider lockfile; only the committed root workspace lock governs dependencies. `cargo metadata --locked --no-deps`, provider tests/doctest and unchanged root-lock checks passed. No provider production behavior changed.
- `6e6d43a`: retain six additional core review regressions for default references, close ordering, acceptance and revocation.
- `0b9b390`: retain the storage gate reacquisition/retirement regression. Together, the seven new tests add no production path or application quota.

The main-worktree `./scripts/check.sh` gate passed all 198 unit/integration tests plus 2 doctests, formatting, workspace all-target Clippy and rustdoc warnings denied, and diff checks. Counts: core 67 (5 unit, 42 persistent owners, 19 memory owners, 1 HTTP), protocol 47, provider 27 and storage 57. Log: `minicore-host-validation/review-regressions-integrated.log`.

The final clean-source check repeated the earlier clone procedure with fixed source `0b9b390750233f7b11ad006563da697014f162e2` into `minicore-host-clean-check-0b9b390`, additionally unsetting `CARGO_TARGET_DIR` and asserting the new checkout's own empty target path. It verified one Runtime package from the published Git source, no active local patch/vendor path/member lockfile, and then passed the complete 198-test/2-doctest gate. The checkout remained clean and the spec hash was unchanged. Cargo source downloads and the installed toolchain were reused; no existing build target was shared. Logs: `minicore-host-validation/clean-source-gate-0b9b390.log` and `clean-source-metadata-0b9b390.json`.

Targeted independent review checked the metadata gate, both current/pending default references and the memory precheck move. It found no new P1/P2 blocking issue in those changes. Its final exact production snapshot was `e2f9ad7` with only the same seven added test fragments; it used a dedicated empty target and passed core 67, storage 57 plus the storage doctest, formatting, Clippy and rustdoc warnings denied. During earlier comparative checks, a shared review build target reused a stale old-snapshot binary; those results were explicitly excluded and the final dedicated-target run replaced them. The intentional old-source failing precheck test is not a failure of the final source.

The Linux workflow's YAML, official immutable action pin, toolchain/command alignment and Bash syntax were independently rechecked after integration. `actionlint`, `yamllint` and `shellcheck` were unavailable; they and the actual hosted Host workflow are not claimed tested. `git ls-remote --heads origin` returned no Host heads at the final read-only check. No Host ref was published or main merged.

`docs/review-handoff.md` now supplies the cross-conversation handoff in the repository, with exact dependencies, commands, invariants and unfinished surfaces. The requested external review remains pending. Recommended next implementation stage: owner-level Host shutdown/history-save continuation and entity retirement, followed by complete canonical service/transport/observation contracts and the remaining DSH/tool/interaction/context surfaces. No first-version completion or TUI/live-provider/performance result is asserted.

## Subsequent independent review correction

The foundation checkpoint did not finish project development or establish full review approval. A later review found an HTTP 200 SSE first-frame rate-limit error discarded the response Retry-After header. The real pre-fix reproducer sent a second request about 3 milliseconds after a 60-second lower bound, despite a 120-second original model deadline. Topic tests `dc14afd` intentionally reproduce this red case; minimal fix `f554761` and documentation `ac565802` are integrated as `d97eccd`, `7b24ca2` and `38991d3`.

The fix only preserves response status/retry metadata for the existing structured error classifier; Runtime remains the sole retry owner. Nine additional real TCP/Runtime cases and a test-only Tokio clock feature cover timing, 30-second cap, remaining original deadline, cancellation, primary error and Started/Unknown/permanent safeguards. At exact integrated `38991d37bc4980fc31a3312e943a62a060ce635a`, the full gate passes 207 tests plus 2 doctests, all lint/doc checks and committed-range whitespace. The root lock is unchanged. A separate provider review rebuilt old and fixed archives independently, reproduced the old failure, passed the new full provider suite and six more probes, and repeated each controlled-clock test 100 times without a failure. These remain scoped provider evidence, not full first-version acceptance.

`5ac4d08` separately removes the two committed manifest EOF blank lines missed by the old clean-worktree-only gate. The new quality helper checks all committed development from the project root baseline, then staged/unstaged changes; shallow history is explicitly rejected and CI fetches full history. Five isolated Git fixture cases and the complete workspace gate passed. No remote Host changes were made. The next lifecycle/history/retirement stage continues in topic worktrees.
