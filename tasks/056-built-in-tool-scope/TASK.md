# Task 056: Record the first-version built-in tool scope

Follow AGENTS.md. This is a scope check, not a new tool implementation or a
promise to preserve minicore-agent's old tool registry.

## Fixed source and actual registration

Checked Host code `e3d634a2b0cd9e662d8efc26284c85af82edd8e5`. Production
`HostResourceBuilder::builtins` in `crates/host-resources/src/lib.rs` registers
`AgentToolFactory`, then `ToolRegistry::register_local_tools`. The latter uses
`crates/host-local-tools/src/schema.rs::local_tool_specs` directly.

| Tool | Current production scope |
| --- | --- |
| agent | Registered six-action owner-bound collaboration tool. |
| read | Registered workspace file/directory reading. |
| write | Registered atomic UTF-8 file writing. |
| edit | Registered literal replacement in an existing UTF-8 file. |
| bash | Registered real Unix shell/process-group execution and cleanup. |
| apply_patch | Not implemented, registered or advertised by this Host. |

Registration does not enable every tool in every Session. Actual selection and
frozen Turn binding still use the existing registry and the same declarations
as tool.list. An absent selection is not silently replaced with another tool.
`host-local-tools/tests/files.rs::model_cannot_supply_owner_identity_and_unknown_tool_is_absent`
currently asserts apply_patch is absent. This records the current scope; it is
not a permanent prohibition on a future requested tool.

## Specification and reference distinction

The preserved v4.93 SPEC contains no named apply_patch requirement. Section 1
requires retaining ordinary tool capabilities; section 7 specifies the existing
Runtime Tool interface, static first-version registration, independent modules,
truthful results/cleanup and Session selection. Its read/edit/bash example is not
an exhaustive mandated-name list. These requirements do not by themselves make
the old Agent registry a compatibility baseline.

Reference Agent `f36a87366d8e2fe0a1c8216463887902e471c739` does include apply_patch in
`src/tools/mod.rs::KNOWN_TOOL_NAMES`, with code under `src/tools/apply_patch/`.
Its accepted scope is a single existing file; legacy application byte limits and
old behavioral decisions must not be imported as Host requirements. The user's
explicit direction treats Agent as code reference only.

Therefore this stage does not add an unrequested patch parser or claim complete
old-tool-name compatibility. apply_patch remains an explicit unimplemented scope
item in first-version acceptance, rather than being hidden behind edit/write or
counted as tested. Any future implementation must have its own required scope,
actual file/change ownership and regression evidence.

## Verification and limits

Read-only source/SPEC comparison and registry tracing were performed. The normal
complete Host gate at the fixed source already passed 779 unique cases and 4
doctests, including the named existing absence assertion. No separate apply_patch
execution, format compatibility, Add/Delete/Move behavior or original user patch
payload was tested. No production source, dependency, TUI, remote ref or Agent
reference was changed by this check. See task048 and
`docs/executable-cold-history-verification.json` for that exact-source gate.
