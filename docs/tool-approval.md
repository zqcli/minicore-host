# Tool approval

The production Host supports `execution.approval` with three values:

| Value | Read-only tools | Mutating tools |
| --- | --- | --- |
| `ask` (default) | Execute | Wait for one approval per invocation |
| `read_only` | Execute | Return a denied ToolResult without executing |
| `auto` | Execute | Execute without asking |

The value belongs to `AgentConfig.session_defaults.execution` and
`SessionConfig.execution`, not Role. New Sessions copy their selected Agent
configuration once. Existing Sessions do not continuously inherit later Agent
changes.

## Configuration and migration

Omitting `approval` means `ask`, including in older saved execution settings.
Serialized/query configurations include the explicit value. Old sample and
unattended configurations that previously relied on the missing production
policy must deliberately add `"approval": "auto"` if unattended execution is
wanted. There is no fallback to Auto when no operator is connected.

For example, a complete execution value with unattended execution is:

```json
{
  "approval": "auto",
  "prompt_timeout_ms": 30000,
  "model_timeout_ms": 600000,
  "policy_timeout_ms": 30000,
  "tool_timeout_ms": 1800000,
  "model_retry_attempts": 2,
  "model_retry_base_delay_ms": 100,
  "compaction": {
    "enabled": true,
    "trigger_percent": 95,
    "target_percent": 50
  }
}
```

`execution` follows the existing whole-field replacement rules. A Session
update supplies the entire execution value under `changes.config.execution`,
with the normal `expected_config_revision`. A partial execution object containing
only `approval` is invalid. Omitting the whole execution field leaves it
unchanged; explicitly setting it to null is invalid. Replacing the entire value
while omitting its `approval` member chooses Ask rather than retaining an earlier
Auto value. Invalid/null approval values are rejected.

The committed configuration is saved and queried through the existing owners.
A public Turn freezes its policy together with the original model, tools, Role,
workspace and config revision. Changing the Session configuration affects the
next Turn, not the current Turn or its internal continuation Loops. A previously
accepted `allow_once` answer is not a standing permission or a policy change.

## Which operations require approval

- The compiled built-in `read` is read-only.
- `write`, `edit`, and `bash` are mutating. Bash is always classified this way,
  even if a particular command appears to only read.
- The compiled built-in `agent` tool uses the same validated action parser as dispatch. `inspect`
  and `wait` are read-only. All valid `create`, `send`, `update`, and `control`
  branches are mutating. This includes creation with initial delegated work,
  both ordinary and delegated sends, metadata/execution-policy updates, and
  cancellation or closure. No action bypasses approval merely by using `agent`.
- Other statically registered, selected tools are conservatively classified as
  mutating because their effects are unknown. Ask requires approval, ReadOnly
  denies them and explicit Auto permits them. A model-invented unregistered tool
  is not made available by this rule. External factories named `read` or `agent`
  follow this same mutating rule; a public name or matching declaration cannot
  claim a built-in exemption.
- Invocation/declaration name mismatches and invalid agent actions cannot be
  approved through this policy. Existing tool schema/argument validation still
  applies. `apply_patch` is not a built-in Host tool and is not advertised.

This is execution control for the Host's trusted statically compiled tools, not
an OS sandbox or a restriction on authorized clients' Host management APIs.
ReadOnly tools and model execution still produce ordinary owner/history records.
Approval does not grant access that the existing owner capabilities deny.

## Existing interaction flow

The registry preserves only the source facts needed for these two built-in
exemptions. `HostResourceBuilder::builtins()` uses the compiled implementations.
Embedders constructing a registry use `register_local_tools()` and
`register_agent_tool(scopes)` for those implementations. Generic `register()`
always records an external factory. No permission metadata or trust platform is
added, and trusted external Rust code is not sandboxed.

The production resolver installs Runtime's `ToolPolicy`; it does not add another
approval service or task table. Ask returns Runtime's existing approval request
with Medium risk. The existing `interaction.requested` event and
`session.state.pending_interactions` identify the original call. Read
`toolcall.get` for the original invocation when deciding; the approval prompt
does not duplicate tool arguments or file contents.

Use the existing `interaction.answer` method with the target Session and the
original `turn_id`, `load_id`, `interaction_id`, a new `operation_id`, and:

```json
{ "kind": "approval", "decision": "allow_once" }
```

or:

```json
{ "kind": "approval", "decision": "deny" }
```

Only the current operating connection may answer. Crossed or stale identities
are rejected. An answered receipt confirms the waiting point was answered, not
that the tool succeeded or the Turn completed. AllowOnce permits that invocation
only; each later mutating call still asks. Deny returns a native denied result
without executing the tool. Cancellation and force shutdown settle the original
wait through the existing Runtime/owner path and do not execute an unanswered
call. Existing deadlines and cleanup ownership are unchanged.

## Implementation and verification

The implementation reuses the three-mode policy/default and single-call approval
pattern from minicore-agent `f36a87366d8e2fe0a1c8216463887902e471c739` as code
reference. It deliberately uses Host's actual six-action agent parser and static
registry behavior rather than copying the old Profile model, unknown-name
blanket denial or unimplemented tool names. Runtime remains pinned to
`69faf9196991524b5549499d6208a1eaf567bf56`; TUI is unchanged.

[Task 077](../tasks/077-tool-approval/TASK.md) records exact commits and test
commands. Acceptance includes real HostResourceBuilder/Host/Runtime external-tool
counter probes and actual Host-binary stdio/WSS, disk and shell-admission tests
against an explicitly scripted local HTTP/SSE provider. These are not live-model
or TUI validation, and they do not substitute for the combined first-version
gate or independent review.
