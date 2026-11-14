# MiniCore Host executable

`minicore-host` assembles the existing Host owners, canonical persistent storage,
production resource preparer, stdio and authenticated WSS transports. It is one
local process, not a daemon manager. TUI is unchanged.

## Start a Host

```sh
cargo run --locked -p minicore-host -- --data-root /absolute/host-data --stdio
```

`--data-root` is required. The storage owner exclusively acquires its actual
canonical directory before executable owners are created. A fresh root writes
its real Host identity and complete `HostConfig`; reopening preserves Host ID
and configuration while generating a new incarnation. Unfinished execution and
queues are not reconstructed. Another process cannot execute against that root.

To initialize resources, supply `--config /absolute/host-config.json`. This is
an initial `HostConfig` JSON document, not a second settings format. The file must
parse when supplied; existing saved configuration wins over these initial values.
Subsequent `host.reload` uses the existing operation-checked owner and production
preparer. Nothing watches or silently rereads the initial config file.

For example, this **template** declares one OpenAI-compatible provider:

```json
{
  "name": "Local Host",
  "framework_instructions": "",
  "models": [{
    "model": "provider/chat",
    "api": "chat_completions",
    "endpoint": "https://provider.example.invalid/v1/chat/completions",
    "provider_model": "replace-with-provider-model",
    "api_key_env": "MINICORE_MODEL_KEY",
    "context_window": 128000,
    "supported_reasoning": ["auto"],
    "supports_tools": true,
    "request_timeout_ms": 120000
  }],
  "tools": [],
  "workspace_roots": [],
  "network": {"listen": null, "access_key_env": null},
  "logging": {"level": "info", "output": {"kind": "stderr"}}
}
```

Use the provider's actual address, model and capabilities. Credential values
come only from the environment sources named in effective configuration and
are private preparation inputs, not saved Host JSON. The process does not scan
arbitrary environment variables, call a model at startup, or install tools.
Selected tools must exist in the compiled production registry.

`api` is required and accepts `"chat_completions"` or `"responses"`. To use
Responses, select `"api":"responses"` and the provider's full Responses endpoint.
The Host does not infer a protocol from the endpoint or fall back to another
protocol. Both ordinary Turns and compaction use the configured selection.
Responses sends complete input with `store:false`; provider-side conversation
state is not required. See [the Responses template](../../example.responses.host.json).

The builtins are `read`, `write`, `edit`, `apply_patch`, `bash`, and the six-action
`agent` tool. Registration does not enable tools:
`HostConfig.tools` selects available definitions and each Session's `tools`
selects its bound set, both using entries such as `{"name":"read"}`. The
template's empty list intentionally enables none. Local tools require a bound
workspace; they do not use the process's current directory as a fallback.
`apply_patch` accepts `{ "path": "relative/file", "patch": "..." }` for one
existing UTF-8 file, using unified diff or a single Codex Update File patch.
It follows the same approval, mutation guard and edit-journal path as `edit`.

`workspace_roots` explicitly enables directory selection before a Session exists.
For example, set `"workspace_roots":["/home/user/projects"]` (or the actual absolute
Windows path, such as `"C:\\Users\\User\\projects"`). Every root must already be a
directory; preparation fixes its canonical path. The default empty list exposes
none and does not fall back to HOME, cwd or a Session workspace. The three
`workspace.roots/list/mkdir` operations use this scope; existing Session workspace
reads and tool execution keep their own configured workspace. Roots can change
through the existing whole-candidate `host.reload` boundary. A browser's selected
path is a path on the Host machine.

Session `execution.approval` defaults to `"ask"`, including when omitted.
Mutating calls request approval with `allow_once`/`deny` answers; builtin
`read` and `agent` inspect/wait do not. `"auto"` and `"read_only"` are explicit
alternatives. The policy stays frozen through the current Turn. See
[resource assembly](../host-resources/README.md) for policy and binding details.

Production bash children omit configured model/network credential environment
names, including previously published names for this Host lifetime. This is
name-based environment exclusion, not output redaction or same-user OS
isolation; already running children are unchanged. See the
[local-tool boundary](../host-local-tools/README.md).

The default configuration permits management but has no model selections. Create
Agents and Sessions using canonical RPC with a configured model. On stdio, send
one UTF-8 JSON-RPC object per LF-terminated line and first complete `initialize`
with `{}`. Stdout is reserved exclusively for protocol frames, including during
startup failure, help, panic and shutdown. `--help` writes only to stderr.

## WSS and client ownership

Set `network.listen` to the desired socket address and
`network.access_key_env` to the access key's environment variable name. Supply
both `--tls-cert /path/chain.pem` and `--tls-key /path/key.pem`. The certificate
chain and matching private key are process inputs; there is no plaintext WS,
anonymous fallback, self-signed certificate generation or disabled TLS validation.
A network listener may run alone or alongside `--stdio`.

Every WSS peer, including loopback, must initialize with its access key before
business access. The first actual initialized stdio or WSS connection receives
the operation right; later connections are read-only. The process supervisor
has a narrow non-serializable lifecycle capability and never takes a hidden
client/operator slot. Disconnect ends only that connection. Existing observers
do not automatically become operators; a later initialized connection may.

Port zero is supported. The `host.started` diagnostic with
`component="host.process"` includes the actual `listen_port`, without logging
configured addresses, paths or keys. The owner startup record is a separate fact.

## Controlled exit

- `host.shutdown` is a normal operation-checked RPC. `graceful` drains accepted
  work; an explicit `force` can upgrade it. Both wait for the real owner cleanup
  before the process exits, not merely for cancellation delivery.
- SIGINT and SIGTERM request graceful shutdown through the process capability.
  Repeated signals remain graceful. No grace deadline escalates to force.
- Stdio EOF or an I/O/framing failure closes that connection, releases its
  operation right, and leaves the Host and accepted work running. Use RPC or a
  signal for controlled exit. A transport failure is reported as a nonzero process
  exit after eventual controlled cleanup, without cancelling accepted work.
- Explicitly paused backlog can keep graceful shutdown draining. An original
  required save/cleanup failure keeps its actual stopping state and data-root
  lease. Repeated shutdown does not repair or retry that original save.
- Queries and explicit force remain usable while draining. Persistent Sessions
  are not implicitly closed or deleted. External kill/crash is abnormal and is
  never reported as successful stopped cleanup.

After Host reports actual stopped, the process closes listener tasks, then ends
its runtime without waiting for Tokio's uncancellable parked stdin read. Only
then does it ask the single diagnostics worker for a one-second best-effort
flush. This diagnostic budget is not a business timeout or persistence proof.

## Diagnostics and verification limits

The effective `LogSettings` installs one structured pipeline after root
acquisition. It uses stderr or the exact configured absolute file path, without
fallback, automatic directory creation or rotation. The actual file writer owns
its exclusive lease through real completion, even if bounded flush times out.
Use externally managed stderr for rotation. Initialization failures are safe
static summaries; credentials, config bodies, paths, prompts and panic payloads
are not printed.

Before opening a configured output file, the application resolves its normal
filesystem path and rejects `host.json` and the Host-owned `agents` storage
subtree. This protects metadata and Session history from diagnostic append.
A dedicated directory such as `data-root/logs` is allowed; create it explicitly.
This startup check is not a filesystem sandbox or protection against arbitrary
external hardlinks and concurrent path replacement.

`host.process` events cover actual startup, shutdown requests/state, transport
failures and process cleanup. They complement the direct Host/Agent/Session/Turn,
model, tool, compaction and result-handoff diagnostics; they do not replace them.
Match both `component` and `event` when selecting a boundary: for example,
`host.owner` and `host.process` each report their own actual `host.stopped` fact.

`host.info.logging` reads the installed pipeline's process-wide health, with
`status` and available `dropped_records`/`write_failures` counters. Embeddings
without a bound health handle report disabled with unknown counters omitted.
The real executable binding is covered by [diagnostic_health.rs](tests/diagnostic_health.rs).
Health, process events and successful diagnostic flush do not prove business saves.

The subprocess suite uses the real executable, OS pipes and signals, filesystem
leases, production HTTP adapter against a gated local HTTP/SSE fixture, and
locally generated TLS certificates. It validates identity/reopen, access rights,
accepted-work survival, controlled and abnormal exit, stdout isolation and file
output ownership. It is not live-provider, external-device or TUI validation.
The CI matrix runs the native executable and Runtime integration on Linux,
macOS (Intel and Apple Silicon), and Windows. Bash requires `/bin/bash` on Unix
and Git Bash on Windows; Git queries require the platform's Git executable.
Unix process groups and Windows Job Objects retain child cleanup ownership.
Platform-specific signal/FIFO tests remain scoped to the OS mechanism they test;
ordinary tool, approval, credential, lease, and process tests run on each platform.
Exact runner results and remaining platform limits are tracked in
[`../../docs/ci.md`](../../docs/ci.md).

```sh
cargo test --locked -p minicore-host --all-targets
cargo clippy --locked -p minicore-host --all-targets -- -D warnings
```

Task 043 records the original immutable executable checkpoint; current composed
validation and remaining live-provider/TUI work are tracked in `../../PROJECT.md`.
