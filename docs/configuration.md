# JSON configuration directories

Host 0.12.0 and SPEC 4.11.0 use one explicit configuration directory. Configuration
files are UTF-8 standard JSON: no TOML, comments, legacy-file fallback or automatic
migration. Runtime persistence stays in the separate data root.

## Files and owners

| Path | Meaning | Reader and owner |
| --- | --- | --- |
| `host.json` | Host name, framework instructions, providers, tools, workspace roots, network and logging | Shared `host-config` loader; Host applies values |
| `agents/*.json` | One declared Agent per file | Loader resolves references; Host and Agent owners apply values |
| `roles/*.json` | One reusable Role per file | Loader expands the selected Role into each native Agent configuration |
| `tui.json` | TUI editor, display and terminal preferences | TUI reads, parses and saves locally |
| `<data-root>/` | Host/Agent/Session identities, configuration snapshots and history | Existing durable owners |

Web owns its `web.json` in its own deployment. It may connect to multiple Hosts.
Host does not parse either UI file or expose a general configuration-file API.
`host.config.get` returns Host-owned values; it does not return frontend files.

`host.json` is required. Missing `agents/` or `roles/` means an empty declaration
set. All JSON files in those directories are parsed; unknown fields, invalid
values, duplicate Agent keys and missing Roles fail validation. Other root files
are ignored.

## Startup

Both executables accept `--config-dir PATH`. Relative paths are resolved against
the startup working directory. The default is `config/` next to the executable.
`--data-root PATH` optionally selects another storage location; otherwise storage
is `<config-dir>/data`. There is no home-directory or legacy environment fallback.

```sh
minicore-host --config-dir /absolute/minicore-config --stdio
minicore-tui --config-dir /absolute/minicore-config
```

TUI embeds Host and passes the resolved directory into its Rust startup API.
It does not launch a Host subprocess or translate shell arguments. TUI reads
`tui.json` itself. Its input history lives at `<data-root>/tui/history.jsonl`.
Only one running process can own a data root at a time.

Copy the entire [example-config](../example-config/host.json) directory and edit
the provider endpoint, provider model and capabilities before starting Host.
Supply the credential through the environment variable named by `api_key_env`.
The examples contain placeholders, never a working key. The sample selects no
tools or workspace; add those explicitly before local file or shell operations.
For an interactive TUI, set `logging.output` to a file with an absolute path
and an existing parent directory. TUI's first-run initializer creates example
Host, Role, Agent and UI files without replacing existing files; its Host example
uses a file log and explicitly records the initial workspace.

Each directory startup prepares providers, logging and network from that
directory's current values. Saved snapshots supply durable identities and
history, rather than overriding current source files. Startup does not send an
Agent message or execute a model call.

The former Host `--config`, TUI `--home`/`--host-config`, `MINICORE_HOME`,
`config.toml`, `agent-defaults.json` and `/agent-defaults` entry points are removed.
Arrange existing settings into the new files explicitly.

## Agent identity and Role references

The complete Agent example is [agents/assistant.json](../example-config/agents/assistant.json).
Its `key` is the stable declaration identity within one Host; `name` is the
display name. Neither identity depends on the Agent filename. Its `role` field
is exactly one filename stem: `"coding"` selects
[roles/coding.json](../example-config/roles/coding.json). It is not the Role's
display name, a path or an ordered search across directories.

First application of a key creates one persistent Agent and a default Session.
Later startup or reload reuses that Agent ID, even if its file or display name
changes. Removing the declaration marks its source unmanaged and retains the
Agent, source key, Sessions and history. Reintroducing that key reuses the same
Agent. Declared Agents expose `configuration_source: {key, managed}` on owner
facts; this metadata is not part of copyable `AgentConfig`.

While `managed` is true, ordinary Agent name/configuration updates and deletion
are rejected. Edit the declaration and reload instead. Start, stop, default
Session selection and explicit Session updates remain available. Dynamic Agents
are independent. TUI `/agent-new <name>` copies the selected Agent's native
configuration into a new dynamic persistent Agent, without its source key,
Session overrides or history. Without a selected Agent, first add a declaration
and reload.

## Three instruction layers

The instruction fields are `host.json.framework_instructions`, Role
`instructions`, and Agent `instructions`. Each may be omitted or `""`; JSON
`null` is invalid. Nonempty layers become separate System messages in the order
Host, Role, Agent, Workspace, then conversation history/input. Role name and
description are metadata. Tool selection remains in the tool fields.

Session creation copies Role and Agent instructions. Changes to these Agent
defaults affect new Sessions; existing Sessions require explicit updates.
Host framework changes apply to future Turns, including existing Sessions.
An active Turn keeps its frozen configuration. No model request rereads a file
or follows mutable Agent values. These layers do not create protocol permission
levels.

## Explicit reload

`host.reload` accepts only `operation_id` and `expected_revision`. It rereads the
directory bound at startup. Callers cannot submit a replacement directory or
configuration body. An embedding without a bound directory rejects this RPC.
Programmatic prepared-configuration APIs remain separate for such embeddings.

The owner validates all files, references and declared Agent resources before
mutation. Changes to restart-only logging or listening fields return
`restart_required` without applying either Host or Agent changes. Otherwise the
Host publishes its configuration, then each direct Agent owner applies its own
change. There is no transaction or rollback across owners.

Follow the original receipt through `preparing` and `applying` to its actual
terminal result. `applied` means all declared changes completed;
`partially_applied` reports each real Agent result after Host publication.
Creation retains the existing separate Agent and Session receipt steps. A
failed result is not retried automatically. Accepted work continues after a
caller disconnects and participates in the Host shutdown barrier.

See [SPEC.md](../SPEC.md) for the normative contract and
[host-config](../crates/host-config/README.md) for the shared loading API.
