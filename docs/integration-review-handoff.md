# Owner, observation and resource integration handoff

## Fixed review target

Published branch: `dev`. Fixed review target:
[c6d8a5e1cf1ea52c8e1e4c104efba3774efc2f5e](https://github.com/zqcli/minicore-host/commit/c6d8a5e1cf1ea52c8e1e4c104efba3774efc2f5e).
Compare with previously published `7900d21a8cc767f508d45a1687ade4376bc6dded`.
Frozen source `705a3e8cfc14d56e51fc08e522df04ee0adca1f6` and the published
head have identical tree `9ff9f0df4474ed75e1df3712c5e8bc7b4d46c3b9`.
Reviewed source code `a5c4bf4451123d994bdd7cceb13787e94bbbcb99` maps to
remote `4186c3d02954dabb43563b0bb0850a70db9db36e`; the later source/head
delta is handoff documentation. The [64-commit publication mapping](owner-integration-publication.json)
resolves all source hashes while retaining the original published history.
Exact-head [CI run 42650587425](https://github.com/zqcli/minicore-host/actions/runs/42650587425)
and quality job `129275534182` completed successfully, including the full quality gate.
Do not use an unqualified latest-dev ref in place of this fixed target.
The original foundation handoff remains [historical evidence](review-handoff.md).

This is a reviewed engineering checkpoint, **not the complete v4.93 release**.
Read AGENTS.md, complete SPEC.md, PROJECT.md and the method coverage document.
The specification SHA-256 remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
No main merge is part of this handoff. Published history must remain intact.

## Included scope

- Real Session-scoped event/output/history observation, UTF-8 catch-up, actual
  retry and interaction state, and 34 shared typed/stdio/WSS routes.
- The six-action agent tool, trusted public-Turn binding, independent delegation
  result handoff before reclamation, normal-queue notifications and scope cleanup.
- Durable Agent name/Session title updates, actual Host resource preparation,
  frozen model/tool/workspace/Role configuration, resource repair and queries.
- Request-only interpretation of old missing tool results, without fabricated
  ToolResults, tool replay, or mutation of the original native transcript.
- Public-Turn ownership of actual tool cleanup, independent cleanup diagnostics,
  and first-terminal-claim preservation across required save I/O.

Trace task018 through task029 and task033/034. The earlier 55 topic commits are
mapped in [integration-source-map.json](integration-source-map.json). Subsequent
source/integration mappings are recorded in task029. Commits are granular and
new subjects use `type(scope): summary`; already published subjects are not rewritten.

## Dependency and execution identity

All six Runtime declarations and the lockfile use actual published Git
`69faf9196991524b5549499d6208a1eaf567bf56`, exactly `0.10.0`, without patch or
path overrides. [Runtime publication evidence](runtime-retry-publication.json)
records its verified trees and all seven successful jobs in CI run42624211012.
Agent code reference is `f36a87366d8e2fe0a1c8216463887902e471c739`, not a
compatibility baseline. TUI remains unchanged at
`3beeb1d8a525e58ce6352e05061f242c5580e248`. Agent and TUI actually pin Runtime
`b276d9c0a1f3a3c06e32029f831ccd100c9699c7`; no joint integration is claimed.

## Exact validation and closed findings

At a5c4bf4, both integration and independent clean-tree validation pass:

```sh
./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
git diff --check main..HEAD
sha256sum SPEC.md
```

The count is **506 unique target/test-name cases plus 3 doctests**. The script
includes committed/staged/worktree whitespace, formatting, all-target Clippy
with warnings denied, tests and strict rustdoc. Rust is pinned to1.90.0.
Nested storage subprocess output is not counted twice. Tests use real owners,
Runtime, temporary files, processes, pipes and local HTTP/SSE/TLS fixtures with
explicitly scripted model responses, not a live online model or TUI.

Prior independent findings were fixed and rechecked: authoritative partial
history, interaction deadline classification, pipe teardown ordering, delegation
handoff/accepted shared-input ranges, and resource repair/query boundaries.
The final task024 lock failure was traced to a hash subprocess inheriting another
parallel test's flock. Only that fixture now hashes in process; production locks,
normal concurrency and all byte assertions remain unchanged. The exact correction
passes 495/3 separately, with 200 producer and 100 independent repeated suites.
The older unrelated storage flake remains explicitly undiagnosed.

Real-Runtime terminal probes additionally exposed failure-before-save and
cancel-before-cleanup-failure races. The fixed claims pass all three committed
regressions, eight actual-process cleanup cases and five independent owner/queue
probes. See task033/CLAIM.md and task029 for exact source commits and log locations.
Scoped review results do not substitute for the requested external final review.

## Not included or not verified

Canonical reload receipts/routes, manual/automatic compaction, ordinary
read/edit/write/bash production wiring, canonical workspace routes, remaining
52-method controls/deletion, complete input-source prompt context, and the actual
Host executable/TLS/logging assembly are unfinished or on separate topics.
Enabled compaction is currently rejected honestly, including in default production
configuration unless explicitly disabled. Ordinary local tools remain excluded
pending their separate corrections and review. No capacity/performance, authorized
live-provider, production-process or TUI acceptance is claimed.

Keep direct-owner saves and necessary cleanup authoritative. Repeated shutdown
does not retry saves, and there is no new unpublished-object recovery endpoint,
last-Arc self-cycle, external-filesystem isolation system or old Agent migration.
Continue incremental implementation on dev and isolated topics; merge main only
after the required independent review and full first-version acceptance.
