# JSON configuration directories

Host 0.12.3 and SPEC 4.11.2 use one explicit configuration directory. Configuration
files are UTF-8 standard JSON: no TOML, comments, legacy-file fallback or automatic
migration. Runtime persistence stays in the separate data root.

## Files and owners

| Path | Meaning | Reader and owner |
| --- | --- | --- |
| `host.json` | Host name, framework instructions, providers, tools, workspace roots, network and logging | Shared `host-config` loader; Host applies values |
| `agents/*.json` | One declared Agent per file | Loader resolves references; Host and Agent owners apply values |
| `roles/*.json` | One reusable Role per file | Loader expands the selected Role into each native Agent configuration |
| `tui.json` | TUI editor, display and terminal preferences | TUI reads, parses and saves locally |
| `<data-root>/` | Owner identities, configuration references, independent Session settings and history | Existing durable owners |

Web owns its `web.json` in its own deployment. It may connect to multiple Hosts.
Host does not parse either UI file or expose a general configuration-file API.
`host.config.get` returns Host-owned values; it does not return frontend files.

`host.json` is required. Missing `agents/` or `roles/` means an empty declaration
set. All JSON files in those directories are parsed; unknown fields, invalid
values, duplicate Agent keys and missing Roles fail validation. Other root files
are ignored.

## Startup

Both executables accept `--config-dir PATH`. Relative paths are resolved against
the startup working directory. The default is `~/.minicore/config`.
`--data-root PATH` optionally selects another storage location; otherwise storage
is `~/.minicore/data`, independent of `--config-dir`. Missing home-directory information is an error; no executable-directory fallback is used.

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

The starter log is `~/.minicore/logs/host.log`; its parent is created during
initialization. Custom config/data paths do not implicitly relocate that log.
Edit `host.json.logging` to choose another absolute destination.

Each directory startup prepares providers, logging and network from the current
source files. File-managed Host and Agent data contain no copied configuration
body. Their stable IDs and fingerprints identify the applied values; Session
configuration and history remain independent. Startup does not send an Agent
message or execute a model call.

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
changes. Owner queries expose `configuration_source: {key}` for file-managed
Agents; dynamic Agents omit it. Source metadata is not part of copyable
`AgentConfig`.

A saved file-managed Agent must have its declaration in the selected directory.
Removing it makes startup or reload fail before any configuration publication or
record write. A failed running reload retains the previous effective values;
restart cannot restore the deleted default configuration from a digest, Session
or history. Restore a valid declaration with the same key to reuse the identity.
There is no automatic unmanage, fallback, detach or snapshot-recovery API.

Ordinary Agent name/configuration updates are rejected for file-managed Agents.
Edit the declaration and reload instead. Explicit deletion remains available
with the existing lifecycle and exact child scope; it never deletes source files.
To remove a declared Agent permanently, remove its declaration and explicitly
delete the Agent before reloading. Keeping a declaration after deleting the
object creates a new Agent through the normal creation path on the next reload.
Start, stop, default Session selection and Session updates retain their behavior.

Dynamic Agents are independent. TUI `/agent-new <name>` copies the selected
Agent's native configuration into a new dynamic persistent Agent, without its
source key, Session overrides or history. Without a selected Agent, first add a
declaration and reload.

## Three instruction layers

The instruction fields are `host.json.framework_instructions`, Role
`instructions`, and Agent `instructions`. Each may be omitted or `""`; JSON
`null` is invalid. Nonempty layers become separate System messages in the order
Host, Role, Agent, Workspace, then conversation history/input. When the selected
tools include the builtin `agent`, ordinary requests also receive the current
Host, Agent and Session IDs after Workspace and before history. The owner-bound
IDs are read-only target facts, not permission grants or user configuration;
they are not stored in history. Compaction requests do not receive this execution
context. Role name and description are metadata. Tool selection remains in the
tool fields.

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

## Persistent files and format boundary

| Data-root path | Contents |
| --- | --- |
| `host.json` | Host ID, configuration source and revision |
| `agents/<AgentId>.json` | Agent ID, Host ID, configuration source, mode, revision and default Session |
| `sessions/<SessionId>/session.json` | Session ID, Agent ID and independent effective configuration/metadata |
| `sessions/<SessionId>/history.jsonl` | Original native history records |
| `tui/history.jsonl` | Frontend input recall, owned by TUI |

Object schema is 3; history schema remains 3. Old object schemas and the nested
Agent/Session layout are rejected without rewriting or migration. Keep an old
data root as a backup and select a fresh `--data-root` for this format; merely
moving files is not a supported conversion.

`data.configuration` is exclusive: a file-managed Host stores
`{"source":"file","fingerprint":"<sha256>"}`, and a file-managed Agent also
stores `key`. Standalone programmatic Hosts store
`{"source":"inline","config":{...}}`; dynamic Agents store
`{"source":"inline","name":"...","config":{...}}`. Constructors do not
convert between these sources. File records contain no duplicate names,
instructions, models or Role bodies.

Fingerprints use SHA-256 over deterministic JSON of the full typed HostConfig or
`[Agent name, resolved AgentConfig]`, including Role content. Input JSON field
order/formatting does not change revisions. Changed effective defaults increment
the Agent revision; an unchanged restart preserves it. A digest cannot recover
configuration contents. Session settings are not inferred back into Agent defaults.
