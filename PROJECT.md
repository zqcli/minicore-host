# MiniCore Host

Implementation of the v4.93 design in `SPEC.md`. This repository is under active development and is not yet a complete or release-ready implementation.

## Baselines

| Component | Fixed commit | Status |
| --- | --- | --- |
| Host spec | SHA-256 f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3 | Complete source copied unchanged |
| Runtime dev | 3fd5ecda1fb7c717ab0a39218d90268b4d97909c | Source reference; integration pending |
| Agent dev | f36a87366d8e2fe0a1c8216463887902e471c739 | Source reference only |
| TUI dev | 3beeb1d8a525e58ce6352e05061f242c5580e248 | Unchanged; integration pending |
| Agent/TUI actual Runtime lock | b276d9c0a1f3a3c06e32029f831ccd100c9699c7 | Not the Runtime dev head |

## Current stage

Foundation, public contracts, storage, owner boundaries and the first real Runtime loop. All 52 resource methods, WSS, DSH, and TUI integration remain part of the full first-version scope; a foundation checkpoint is not full completion. The fixed Runtime needs the design's authoritative incremental-history handoff and partial-output report before full persistent multi-step execution can meet the design.

Implementation runs on isolated local branches/worktrees. `main` holds the reviewed baseline only; active work is integrated on `dev`. Remote publication awaits explicit approval. Independent review is required before a main merge.

## Execution provenance

Workers were requested with `gpt-6-astra` and `max`. The execution interface does not expose an independent actual-model/effort attestation; requested parameters are not claimed as verified runtime identity.

## Validation

See `tasks/001-foundation/TASK.md`. Nothing in the initial baseline represents a successful build, provider call, or TUI integration.
