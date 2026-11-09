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
    "endpoint": "https://provider.example.invalid/v1/chat/completions",
    "provider_model": "replace-with-provider-model",
    "api_key_env": "MINICORE_MODEL_KEY",
    "context_window": 128000,
    "supported_reasoning": ["auto"],
    "supports_tools": true,
    "request_timeout_ms": 120000
  }],
  "tools": [],
  "network": {"listen": null, "access_key_env": null},
  "logging": {"level": "info", "output": {"kind": "stderr"}}
}
```

Use the provider's actual address, model and capabilities. Credential values
come only from the environment sources named in effective configuration and
are private preparation inputs, not saved Host JSON. The process does not scan
arbitrary environment variables, call a model at startup, or install tools.
Selected tools must exist in the compiled production registry.

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

Port zero is supported. The `host.started` diagnostic includes the actual
`listen_port`, without logging configured addresses, paths or keys.

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

Process events cover actual startup, shutdown requests/state, stopped cleanup,
stdio disconnect, transport failure and owner shutdown faults. These are not
complete Host/Agent/Session/Turn instrumentation. Logging health is not yet wired
to `host.info`; process events and a successful flush do not prove business saves.

The subprocess suite uses the real executable, OS pipes and signals, filesystem
leases, production HTTP adapter against a gated local HTTP/SSE fixture, and
locally generated TLS certificates. It validates identity/reopen, access rights,
accepted-work survival, controlled and abnormal exit, stdout isolation and file
output ownership. It is not live-provider, external-device or TUI validation.
The tested platform is Linux; the repository's storage implementation is Unix.

```sh
cargo test --locked -p minicore-host --all-targets
cargo clippy --locked -p minicore-host --all-targets -- -D warnings
```

Task 043 records immutable validation checkpoints and remaining integration work.
