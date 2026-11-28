# JSON configuration directories

Host 0.13.0 uses one explicit configuration directory. Files are UTF-8 standard
JSON: no TOML, comments, legacy-file fallback or automatic migration. Runtime
persistence stays in the separate data root.

## Files and owners

| Path | Meaning | Reader and owner |
| --- | --- | --- |
| `host.json` | Host name, shared instructions, providers, models, tools, workspace roots, network and logging | Shared `host-config` loader; Host applies values |
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

Copy the entire [example-config](../example-config) directory and edit the
[providers and models](#providers-and-models). Before upgrading an existing
configuration, [migrate the flat model list](#migrate-from-a-flat-model-list).
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

## Providers and models

Declare each upstream connection once in the `providers` object. Two models
can reference the same provider; their `name` values still select different
MiniCore models. This example can replace `host.json` in the example directory:

```json
{
  "name": "Local Host",
  "providers": {
    "gateway": {
      "baseurl": "https://gateway.example.invalid/v1",
      "api": "responses",
      "api_key": "$MINICORE_MODEL_KEY"
    }
  },
  "models": [
    {
      "name": "provider/chat",
      "provider": "gateway",
      "model": "replace-with-chat-model",
      "context_window": 128000,
      "reasoning": ["auto"],
      "supports_tools": true
    },
    {
      "name": "provider/review",
      "provider": "gateway",
      "model": "replace-with-review-model",
      "context_window": 128000,
      "reasoning": ["auto", "high"],
      "supports_tools": true
    }
  ],
  "tools": [],
  "network": {"listen": null, "access_key_env": null},
  "logging": {"level": "info", "output": {"kind": "stderr"}}
}
```

| Field | Use |
| --- | --- |
| `providers.<key>.baseurl` | Upstream API root, including an existing prefix such as `/v1`. Do not include the method suffix. Use HTTPS, or HTTP for a numeric loopback address. |
| `providers.<key>.api` | `chat_completions` or `responses`. Host appends `/chat/completions` or `/responses`, respectively; it does not add `/v1`. |
| `providers.<key>.api_key` | A literal key or `$ENV_VAR`. Prefer the environment reference. |
| `providers.<key>.stream` | Optional and Responses-only: omit or set `sse` for HTTP SSE; set `wss` to try WebSocket first. |
| `providers.<key>.request_timeout_ms` | Optional positive request timeout in milliseconds, shared by models on this provider. |
| `models[].name` / `provider` / `model` | Unique MiniCore selection name, exact provider key, and upstream model ID, respectively. Agent `session_defaults.model` and existing Sessions select `name`, not the upstream ID. |
| `models[].context_window` / `reasoning` / `supports_tools` | The declared context window, supported reasoning levels, and function-tool capability. Set these to match the upstream model. |
| `models[].web_search` | Optional Responses hosted search: `live` allows external web access, `cached` disables it. Omit to leave search off. |

The `$` prefix means the whole value names a variable in the Host process
environment. `${VAR}`, shell commands and interpolation are not supported.
Startup and reload fail if a configured model's credential is missing or invalid.
Literal key bytes are not returned by RPC or written to data records or diagnostics;
they are still present in the source file if supplied there.

Omitted `stream` does not probe WebSocket. `wss` tries a fresh upstream socket
for each Responses request. It can fall back to the same endpoint over SSE only
on supported pre-send failures or an explicit pre-start route rejection; it
does not replay a request that may already be running. This setting does not
control the Host's inbound WSS listener. See the
[Responses adapter](../crates/host-provider/README.md#responses) for exact behavior.

A Session can override the model's search default with `web_search: "off"`,
`"live"` or `"cached"` for its next Turn. Search runs on the upstream Responses
request, not as a local tool or a separate `/v1/search` endpoint.

### Migrate from a flat model list

Edit `host.json` before starting 0.12.18. The old flat rows are not accepted.
Group rows with the same connection settings into one named provider. If their
API, credentials, stream or timeout differ, keep separate providers.

| Old model-row field | New location |
| --- | --- |
| `endpoint` | Provider `baseurl`: remove the `/responses` or `/chat/completions` suffix; keep the API prefix. Convert an old `wss://` URL to `https://` (`ws://` to `http://` for loopback) and set provider `stream` to `wss` if you still want WebSocket. |
| `api`, `stream`, `request_timeout_ms` | Same fields on the provider. Use `stream` only for Responses. |
| `api_key_env: "KEY"` | Provider `api_key: "$KEY"`. |
| `model`, `provider_model`, `supported_reasoning` | Model `name`, `model`, and `reasoning`, respectively. |
| `context_window`, `supports_tools`, `web_search` | Unchanged on the model. Add `provider` referencing the new provider key. |

Keep the old model alias as `name` so existing Agent declarations and Session
model references continue to resolve. Do not change the data root. The new
layout is for file declarations only: `host.config.get` and the Rust embedding
API still expose effective model settings, not the `providers` file object.

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

The instruction fields are `host.json.instructions`, Role
`instructions`, and Agent `instructions`. Each may be omitted or `""`; JSON
`null` is invalid. Nonempty layers become separate System messages in the order
Host, Role, Agent, Workspace, then conversation history/input. When the selected
tools include the builtin `agent`, ordinary requests also receive the current
Host, Agent and Session IDs after Workspace and before history. The owner-bound
IDs are read-only target facts, not permission grants or user configuration;
they are not stored in history. Compaction requests do not receive this execution
context. Role name and description are metadata. Tool selection remains in the
tool fields.

All three declarations use `instructions` for inline text and
`instructions_file` for Markdown. In existing `host.json` files, rename
`framework_instructions` to `instructions` (or remove it if empty).
The old key is not accepted. Embedders also need to rename the Rust field
to `HostConfig::instructions`; if reopening old inline Host records, migrate
the same key under `data.configuration.config` while the Host is stopped.

File examples:

| Declaration | File field | Example value |
| --- | --- | --- |
| `host.json` | `instructions_file` | `prompt/host.md` |
| `roles/lead.json` | `instructions_file` | `../prompt/lead.md` |
| `agents/assistant.json` | `instructions_file` | `../prompt/assistant.md` |

Paths use `/` and are relative to the JSON file that declares them. They must
resolve to a regular UTF-8 `.md` file inside the configuration directory.
Omit the corresponding inline field entirely when using a file. Host reads the
text verbatim at startup and configuration reload. Missing or invalid files
fail configuration loading. Programmatic configurations still use inline text.

Session creation copies Role and Agent instructions. Changes to these Agent
defaults affect new Sessions; existing Sessions require explicit updates.
Host instruction changes apply to future Turns, including existing Sessions.
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
| `sessions/<SessionId>/session.json` | Session ID, Agent ID, independent effective configuration, optional title and first-user preview |
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
