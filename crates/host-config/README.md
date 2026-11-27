# Native JSON configuration

`minicore-host-config` contains readable Host values and configuration-directory
loading. It depends on protocol values, Serde and the filesystem. It has no
Runtime, resource handles, owner state, UI configuration or credential access.
Core re-exports the shared Host types for existing Rust embedding calls.

`resolve_config_paths(config_override, data_override)` makes explicit paths
absolute against the startup working directory. Without an override,
configuration defaults to `~/.minicore/config`; data defaults to
`~/.minicore/data`, independently. Path selection does not create files or consult legacy
`MINICORE_HOME` rules. Native home discovery supplies defaults; missing required
home information fails instead of falling back to the executable or cwd.

`load_config_directory(root)` reads `host.json`, `roles/*.json` and
`agents/*.json`. The Host file is required. Missing Agent/Role directories are
empty declaration sets. Host, Agent and Role declarations can reference UTF-8 Markdown within the
configuration directory instead of inline instructions. Frontend files,
including `tui.json`, are ignored.
Every parsed object rejects unknown fields. A bad file, missing Role reference
or duplicate Agent key fails the whole read with its source path.

Named providers in `host.json` expand into effective model settings referenced
by model `name`. File-only fields do not change the `HostConfig` wire shape.
The loader separates literal keys from serializable settings; resource
preparation resolves credentials. See the
[file format and migration](../../docs/configuration.md#providers-and-models).

An Agent file has `key`, `name`, `role`, optional `instructions`, and complete
native `session_defaults`. Its `role` is one filename stem from `roles/`, not a
Role display name or an arbitrary path. The result contains the canonical
source directory, one `HostConfig`, and key-sorted `{key,name,config}` values.
Role references are expanded into native `AgentConfig.role` values; stable keys
remain separate from copyable Agent configuration. Loading does not prepare
resources, allocate identities, read durable records or publish configuration.
Those operations belong to Host and its direct owners, outside owner locks.

`create_config_directory(root, workspace)` creates example Host, Role and Agent
files with `create_new`, returning only the paths actually created. It never
overwrites existing files. The examples contain a placeholder provider endpoint
and environment-variable name, no actual credentials. The Agent explicitly
selects the canonical supplied workspace or initialization working directory;
the same directory is an explicit picker root in the generated Host file.
Logging goes to the absolute native-home `.minicore/logs/host.log` path; the
initializer creates its parent. Host config remains the owner of custom log paths.

Host, Role and Agent instructions each default to an
empty string when omitted. Native serialization retains explicit empty strings
so strict durable-record reads never discard an existing instruction field.
Session creation copies Role and Agent instructions
from one Agent version. Existing Sessions retain their independent values;
explicit Session updates apply to the next Turn. Nonempty prompt layers remain
separate and ordered Host, Role, Agent, Workspace, then history.
