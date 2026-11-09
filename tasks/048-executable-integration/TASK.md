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
A combined all-target test run excluding the two known-blocked storage and
diagnostics packages is in progress; exclusion is not a whole-workspace pass.

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
