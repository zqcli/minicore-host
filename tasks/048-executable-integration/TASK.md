# Compose production tools, process and workspace

Follow AGENTS.md. Local composition only; no publication or first-version claim.

## Fixed source and boundaries

- Code: `78d1fe24d46ce355409e0899dd7b96e957a558f8`, `stage/executable-next`.
- Base: `f4f03b1cd9ac9d6e139f8e6a457a2e9d7c154e44`, the documented execution checkpoint with an explicitly open diagnostic lease finding.
- Source mapping: `docs/executable-source-map.json`, 26 incremental commits with exact original messages and order inside each topic.
- Production tools source `0a4f90de99655231c45cd1e3d9996460f9c94281`: actual built-ins and shared workspace resources, Session-owned edit observations, original per-call output leases, incremental UTF-8 and completion only after actual worker cleanup. Independent review passed 664 unique cases/4 doctests and the four unchanged plus two extra probes, closing the premature-finished P1.
- Process source `48c62ada9a5a7eb707cd0b68797cd7a9d1f0e973` (code `544fd079c9af8571b4ea545c61db141aa94bd8f1`): trusted supervision, canonical saved configuration, real stdio/TLS startup and explicit graceful/force lifecycle, protected diagnostic destinations. Independent review passed 658/4 and nine additional real-process/lifecycle probes. Actual ordinary tools through this binary were excluded from that topic review.
- Workspace source `fa2b13034894dbc4963b42baec0f1fe18b3a0bf8` (code `4f91fd14645d09703cc325e79f08948054c97753`): six real canonical query routes and actual Session/Turn edit snapshots. ChangeRef preserves its original canonical-root scope; wrong/reconfigured roots fail before Git/content reads, same-root references remain valid. Independent routing/tool review passed original and additional probes, 90 repeated canonical cases, 20 original probe runs and remaining lint/docs. Its aggregate is blocked by the unchanged storage lease failure below; it is not reported as fully green.

No diagnostics source correction is duplicated: source af0f88e was already in the base. Integration conflicts were only the app task note (restored from its original source), adjacent host module declarations (kept both), and public exports (kept both trusted process and workspace types). Selected topic-owned production paths and all raw commit messages were compared against their exact source commits. No business assertion or logic was changed by these resolutions.

## Actual validation and open blockers

`cargo check --workspace --all-targets --locked` passes at the exact composition.
Log: `/workspace/shared/minicore-host-validation/executable-next-workspace-check.log`.
The combined all-target test run excluding the two known-blocked storage and
diagnostics packages passed **610 unique cases** at code `78d1fe2`. The separate
`--list` inventory confirms the count. Logs are `executable-next-unblocked-tests.log`
and `executable-next-unblocked-list.txt` under the same validation directory.
This explicit exclusion is not a whole-workspace pass.

Known aggregate failures are preserved, not dismissed by rerunning until green:

- Diagnostics reports `Flushed`, but immediate alias reopening sees
  `LockOutput(WouldBlock)`. A separate worker reproduced 26 failures in 100 normal
  parallel runs and proved close-only retention through inherited fork FDs.
  Its deterministic production-writer regression fails the unchanged base.
- Storage's real `drop(writer)` followed by `read_history` reports `HistoryInUse`
  in two independent topic aggregates. The workspace review repeated this once
  in ten parallel whole-target runs; isolated runs passed 20/20. A dedicated
  storage task must independently verify and correct its actual ownership paths.

These are active implementation blockers. Existing successful topic gates and
the earlier 650/4 integration gate remain historical evidence only. Necessary
cleanup/save ownership, real file bytes, normal parallel testing and exclusive
acquisition must not be weakened to hide them.

## Next work

Integrate separately reviewed lease corrections, then run the complete fixed-head
gate and actual Host-binary read/write/edit/bash scenarios. Canonical controls,
context, persistent deletion, full input association and business diagnostics
remain separate tasks. Update PROJECT and the method map after validated
composition; no route count alone proves complete contracts. Runtime remains
published 0.10.0/69faf919, SPEC unchanged, TUI unchanged. No main merge or push.

## Six canonical controls integrated

Reviewed control source `570b886fcd4c4315e155d3f244576a681320fa7e` and documentation
`cef0c189e7972d5607d61e0303d8ee7e8ed1c0ff` are integrated through local
`f98af861d1bf618c99b8c28688ee1fb5122dacd5`. Seven source commits applied without
conflicts, bringing the source map to 33 entries and actual shared routes to 49
(including the separately managed `events.subscribe` route). This adds Agent
update/start/default selection, Session update and input list/remove. Independent
review reproduced the synchronous-validation timeout P1, then passed the unchanged
seven probes after validation moved outside the caller poll and before final owner
acceptance. Accepted saves retain their prior owner semantics. Its scoped review
passed; global lease blockers are still open. Context and deletion routes are not
included. The earlier 610-case run predates these seven commits, so it is not
claimed as their aggregate validation.

## Reviewed lease corrections and adapter fixture correction

The two lease findings above are now corrected in this local composition. The
original failures remain historical evidence. Diagnostics source `8fc7db845c642507941c339eb67c83ae6f35a7a6`
plus its red test/docs and storage source `9dd9f7c251506ceeeb1cabebde56a2f58682ec67`
plus its red tests/docs were integrated through `ab80412`; documentation clarification
is `e30c29c0688f303affc85fd20d4b95bbf140d053`. Both corrections passed independent
review, including original deterministic fork-inherited descriptor failures and
real delayed save/cleanup ownership. They explicitly release their own final file
lease only after the required writer work, rather than weakening exclusive access.

The first combined `./scripts/check.sh` at exact `e30c29c` stopped with two failing
transport adapter assertions. Both still expected `agent.start` to be staged, but
the reviewed canonical-controls topic now implements it. Empty parameters correctly
produce JSON-RPC invalid parameters, not HOST_METHOD_UNAVAILABLE. The preserved
aggregate log is `/workspace/shared/minicore-host-validation/executable-lease-combined-gate.log`.
It is not a green workspace gate.

The integration correction changes only those fixture expectations: malformed
`agent.start` returns -32602; every genuinely staged catalog method is checked
against the current supported-method set for the existing 1401 business fault.
Receipt missing/ambiguous checks remain. No production route, permission, fault or
business assertion was removed. After rustfmt, `cargo test -p minicore-host-transport
--test adapters --locked` passed all 12 cases; log
`/workspace/shared/minicore-host-validation/executable-staged-method-fixture.log`.
The first formatting check caught only formatting in the new assertion and was
corrected before this focused run. A new whole-workspace gate is still required
at the final combined checkpoint.

## Context and real-binary acceptance composition

The five independently reviewed context commits through `8257f3a` are integrated
at `1c84db2`; canonical context transport `955f07e` plus docs are integrated at
`2c78839`. Context adds actual committed/bound configuration, retained history
usage and save/compaction facts without implicit loading. Review's two reproduced
P2 findings are fixed: public automatic compaction uses continuous response_index,
and accepted complete/partial/terminal history and original save registration are
published atomically. Source review passed 666 unique cases and 4 doctests, plus
original and additional exact-journal concurrency probes.

Real binary ordinary-tool acceptance source `f74d793` and docs `be8633f` are
integrated through `f7ee4a1f8650ab2fb0aced250ea43e89932b1b93`. Independent scoped review
passed all 17 app cases, 40 repeated permanent tool cases and 30 repeated additional
binary controls. Those controls cover unselected-write rejection, accepted tools
surviving stdio EOF, and exact binary/UTF-8 suffix plus restart history. This is
real binary/Runtime/HTTP-fixture verification, not live remote provider or TUI.

The context route had only adjacent import conflicts; both WorkspaceSnapshot and
SessionContextSnapshot were retained with no handler logic edits. The combined
full gate at `f7ee4a1` stopped during Clippy compilation: its newly imported private
context fixture still called the older seven-argument Session constructor. The
already-reviewed production-tools branch adds a `reopened` argument for edit
coverage. This fixture creates its record immediately before construction, so the
integration fix passes `false`; it does not change production behavior or weaken
an assertion. The failed log is
`/workspace/shared/minicore-host-validation/executable-context-process-f7ee4a1-full.log`.
A fresh complete gate will be run after this explicit fixture correction.

## Fixed combined gate

At `70c17e7226bd90c7bb1e146ed141081f567e9d23`, the complete `./scripts/check.sh` passed
with **761 unique cases and 4 doctests**. The locked `--list` inventory confirms
761 distinct target/name pairs over 84 nonempty targets. Metadata confirms only
published Runtime 69faf919 at 0.10.0; SPEC and lock remain unchanged. Exact log and
metadata hashes are in `docs/executable-verification.json`; current scope and open
query-contract findings are in `docs/executable-review-handoff.md`. The new green
aggregate does not close the independently discovered query projection gaps or
claim deletion, cold-read corrections, provenance or business diagnostics are
integrated. Earlier failed runs and checkpoints are preserved. No remote publish
or main merge has occurred for this stage.

## Reviewed cold-read composition

Task052 source98ce59b and its bootstrap/red-test/docs are integrated without
conflicts through `e3d634a2b0cd9e662d8efc26284c85af82edd8e5`. Independent review
replayed the original active-writer failure and passed 248 core + 28 transport cases,
8 additional authority/lifetime/progress probes, strict lint/docs and 4 doctests.
The normal complete combined gate now passes **779 unique tests and 4 doctests**;
`--list` independently confirms 779. Exact evidence is in
`docs/executable-cold-history-verification.json`; the prior 761-case manifest and
failed runs are not replaced. Query projections, deletion, provenance and full
business diagnostics remain separate unfinished work.
