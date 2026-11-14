# Engineering instructions

- The complete design baseline is `SPEC.md` (v4.11.0, SHA-256 `0fe877e0d2809fd6a11b44ff8f90c6aa9029446f3757ea26d393a2ebed9ef016`). Implement its ownership, execution, and acceptance contracts.
- Develop on `dev` and short-lived topic branches/worktrees. Do not merge into `main` before independent review and required tests pass.
- Follow the repository owner's current explicit publication authorization and applicable permissions. Confirm the exact destination and source checkpoint, preserve remote history, and never force-push.
- Use descriptive English commit subjects in the exact `type(scope): summary` form, for example `feat(core): bind execution resources`. A scope is required. Do not add co-author trailers. Preserve already published commit history.
- Keep `PROJECT.md` and each task's `tasks/<id>/TASK.md` current with scope, exact commits, commands, outcomes, limits, and next steps.
- Keep direct ownership: Host -> Agent -> SessionController. The existing Session task owns and drives the whole Turn, prepares complete model requests, and calls internal Runtime model/tool primitives. Do not add cross-layer mutation shortcuts, capacity quotas, implicit retries, fake success, or fake integration data.
- Keep Runtime in `crates/host-runtime` with a workspace path dependency and the committed Cargo.lock; pin external source dependencies. Runtime must not depend on Host owner crates. Providers remain in `host-provider`. Production results must come from real owners and execution primitives; name test doubles and never present them as live provider or TUI validation.
- Remove the replaced standalone AgentLoop lifecycle, input/history callbacks, mutable loop configuration, and cumulative report reconciliation. Preserve native durable value formats and actual partial results; an internal LoopId correlation field does not restore a loop lifecycle. Ordinary and compaction requests share model-call safety; each compaction model call initially uses one attempt and explicit deadlines.
- Task 111 coordinates Host and TUI configuration changes under SPEC 4.11.0. Keep Runtime internal, pin the exact released Host revision in TUI, and do not modify the independent Web client as part of this task.
- Never hold owner state locks across I/O or child calls. Accepted work must outlive a caller's wait.
- Run formatting, lint, unit/integration tests, and documentation checks. Document checks that were not run or are blocked.
- Count unique test cases with `cargo test --workspace --all-targets --locked -- --list`; nested storage subprocess probe output is not an additional test case.
- Engineering workers for this project are requested with `gpt-6-astra` and `max` reasoning. Record the requested configuration without claiming independently verified effective runtime settings.
- Rust Agent/Session creation uses the existing asynchronous owner entries for both storage modes. Memory metadata updates reuse owner commit methods and remain callable without an executor. Delegation fact types have one core definition; preserve separate capability and lifecycle boundaries.
- Parallel contributors own disjoint paths. Coordinate shared manifests and public types before changing them.
- Use minicore-agent as a code reference and reuse suitable mature components. It is not a compatibility or behavior baseline; follow this Host specification and justify necessary changes without speculative rewrites.
