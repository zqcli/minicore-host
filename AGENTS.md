# Engineering instructions

- The complete design baseline is `SPEC.md` (v4.93, SHA-256 `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`). Preserve its contents and implement its ownership and acceptance contracts.
- Develop on `dev` and short-lived topic branches/worktrees. Do not merge into `main` before independent review and required tests pass.
- Follow the repository owner's current explicit publication authorization and applicable permissions. Confirm the exact destination and source checkpoint, preserve remote history, and never force-push.
- Use descriptive English conventional commits. Do not add co-author trailers.
- Keep `PROJECT.md` and each task's `tasks/<id>/TASK.md` current with scope, exact commits, commands, outcomes, limits, and next steps.
- Keep direct ownership: Host -> Agent -> SessionController -> bound Runtime Turn. Do not add cross-layer mutation shortcuts, capacity quotas, implicit retries, fake success, or fake integration data.
- Use fixed dependency revisions and a committed Cargo.lock. Production results must come from real owners and Runtime. Test doubles must be named as test doubles and must not be presented as live provider or TUI validation.
- Keep TUI unchanged during the foundation stage. Record its pinned baseline and distinguish it from Runtime HEAD.
- Never hold owner state locks across I/O or child calls. Accepted work must outlive a caller's wait.
- Run formatting, lint, unit/integration tests, and documentation checks. Document checks that were not run or are blocked.
- Count unique test cases with `cargo test --workspace --all-targets --locked -- --list`; nested storage subprocess probe output is not an additional test case.
- Parallel contributors own disjoint paths. Coordinate shared manifests and public types before changing them.
