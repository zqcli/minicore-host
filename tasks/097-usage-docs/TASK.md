# Task 097: reconcile current crate usage documentation

## Fixed source and scope

- Base: `c22d8c8af058b756d5f37bc6d1a01f1dc7749934`, tree
  `a805cb4f6bf0d817fdf5d06226f8638e14ba95e6`.
- Branch/worktree: `task/097-usage-docs`, `minicore-host-task097`.
- Read AGENTS, the software-engineering skill, all twelve crate READMEs,
  relevant current implementations/tests and final SPEC contracts.
- Requested worker: `gpt-6-astra` / `max`; effective configuration is not
  independently attested.
- Documentation only: ten existing crate READMEs and this task. Root README,
  PROJECT/handoff, historical tasks, production, manifests/lock, SPEC and TUI
  are unchanged. No new guide, dependency, build target or remote action.

## Corrections made

- **Executable/diagnostics:** replace the obsolete missing `host.info` health
  claim with the actual `process.rs` → `Host::bind_logging_health` →
  `HostInfo.logging` path. Distinguish `host.process` boundaries from owner,
  model/tool/compaction/handoff records, including the two independently owned
  `host.stopped` facts. The startup port belongs specifically to
  `component="host.process"`, not the owner event with the same name.
  Health/flush remain separate from business persistence.
- **Tools/configuration:** document the actual builtin agent/read/write/edit/bash
  registry, explicit Host and Session selection, absent apply_patch, default
  Ask and frozen per-Turn policy. The exemption depends on builtin origin, not
  a tool's name. Credential protection is command environment-name exclusion,
  not output rewriting, retroactive child changes or an OS sandbox.
- **Core/transport/resources:** remove already-completed integration items from
  current limits. Correct the obsolete 43-route list to the actual SPEC resource
  set, distinguish ordinary dispatch from subscription setup, and describe the
  saved-only result branch without inventing old Load/config/execution state.
- **Protocol/provider/compaction:** correct preserved unknown-Fault decoding,
  root-workspace/lock status and stale Runtime revisions. Older provider test
  counts are explicitly historical, not today's fresh test result.
- **Workspace/storage:** replace future write-tool/owner-deletion wording with
  actual component ownership. Native ToolResults are saved; retained edit
  snapshots are not a new persistent diff database.

The local-tool and agent-tool READMEs already describe their current execution
and lifetime boundaries and were left unchanged. Live authorized providers,
actual TUI integration and stronger same-user/OS isolation are not claimed.

## Verification performed

No Cargo/build or full suite was run by this documentation worker. The
coordinator owns the current composed gate. Static verification passed:

1. Parse the existing app HostConfig JSON template; compare its top-level,
   model, network and logging field names with `host-core/src/configuration.rs`.
   Read actual validation and CLI parsing/transport preparation. The template
   and documented flags still match; the static check does not execute Rust
   deserialization. The additional real CLI smoke is described below.
2. Compare the README route names with SPEC §8, the protocol catalogue and
   `operations.rs`: identical resource sets, with 51 ordinary routes and the
   real subscription path. This checks documentation, not completion by count.
3. Check the two documented Runtime pins against their manifests and root lock:
   `3aa47629929906832dcb309afb7d242c1ae3e53f` / 0.10.0.
4. Read current approval, environment-exclusion, logging-health, result codec
   and owner/query implementations; verify referenced test/source files exist.
5. Resolve relative Markdown links, run `git diff --check` and
   `scripts/check-whitespace.sh`, and verify only authorized documentation paths
   changed. SPEC SHA-256 remains
   `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

Supporting previously executed evidence was read, not rerun: the coordinator's
`full-50a1b2f.log`, SHA-256
`e09269d6cd3db15f1b93a80df03e851d6b9a23ce327fd1fa05943d165f48f739`.
It records passing actual-binary logging-health, credential-exclusion and
Ask/read-only/frozen-policy tests, plus saved-result transport and real
workspace-edit/reopen tests. The sampled CLI, health binding/test, policy,
configuration, Fault and operation-table source files are unchanged between
that checkpoint and this base. This does not rename that old run as a fresh
`c22d8c8` gate or this worker's independent runtime validation.

### Actual README-template CLI smoke

At the coordinator's request, ran the existing root debug binary read-only:
`minicore-host-recovery-target/debug/minicore-host`, SHA-256
`1382da6ba0efe36eaf77664600de2120ef4c574e7c534353ccbe33ca3bd54298`.
It is the coordinator's `c22d8c8` full-gate build, not rebuilt here. The retained
`full-c22d8c8.log` starts with that source and was rehashed as
`dfed41bf43bfe032b724be6b83aa3ba243776bb9dd1849769342e97516961b1a`;
later root commits inspected at this point changed only documentation.

The smoke extracted the README JSON unchanged into a fresh temporary directory,
set only its named model credential to an explicit dummy value, and launched
`--data-root <fresh-root> --config <template> --stdio`. Actual `initialize` and
`host.info` succeeded, identity matched, health was `ok`, and saved HostConfig
equalled the template. Stdin stayed open until an explicit SIGTERM; controlled
exit returned 0 and both original owner/process stopped records were observed.
Stdout contained only the two RPC responses. No Agent/Session was created and
no model/tool request was issued. No external-provider validation, environment
dump or real credential use is claimed. Temporary files/processes were cleaned up.

Private reproducible evidence is under `recovery-second/task097`:

| File | SHA-256 |
| --- | --- |
| `check_docs.py` | `f14b2bd2e07614c1dd417edb573ab577beaa437bfff0536c0b535afc2c6de018` |
| `static-check.json` | `2841fa57a644142c06d915c9b26e76bc7c14ec074e261c3c4e215600a0be63fe` |
| `cli_smoke.py` | `152c22c45b1d7f29026d40b804cf9f00e636b64eb93610b439804988240db327` |
| `cli-smoke.json` | `5b3c2a71e949cec58813ec34fb67ffa1248631b036c9571f508513513b3c797e` |

The coordinator owns integration and updates to root-facing summaries. No
production repair or additional compatibility/recovery work was introduced.
