# Production Host resource assembly

`HostResourceBuilder` composes real OpenAI-compatible HTTP/SSE clients, the statically compiled tool registry, canonical workspace bindings, execution deadlines/retries, and one frozen framework/Role prompt per Turn. The built-in registry currently contains `agent`; registration alone does not enable it. HostConfig.tools selects available definitions, and SessionConfig.tools selects the actual ToolSet.

Keep one builder across reloads, so the agent tool's independently owned delegation scopes survive between Turns. `prepare` reads only explicitly named environment variables. `prepare_with_credentials` supports explicit private embedding inputs, including isolated fixture credentials. Neither path sends model requests or executes tools to test availability.

A production setup calls `prepare`, then `Host::memory_configured`; persistent setup calls `Host::open_configured` with the preparation closure. Saved HostConfig values are authoritative on reopen. Provider and network key bytes are never serialized. There is one `HostRecord<HostConfig>` disk shape; the name-only HostSettings convenience does not define another storage format.

For reload, read `Host::config_get` before preparation, prepare a complete candidate without owner locks, then pass it and the original config_revision to `Host::reload_configuration`. A changed listener address or any logging setting returns RestartRequired for the whole candidate. Unchanged startup settings do not block a hot update. The application owns installing its logging backend exactly once and creating its listener; this crate does not do either.

After acceptance, Host owns the original configuration save and publication independently of the waiter's connection/future. Saves run outside owner locks. Version reservation excludes a competing write before it reaches disk. Failed saves preserve the live snapshot/key, retain the candidate and fault, and prevent a false successful shutdown. No new save-recovery interface, automatic retry, rollback, external-file snapshot protocol, or config migration is added.

Successful publication replaces one immutable resource snapshot and changes network authentication at the same boundary. Only actual network key changes revoke old network connections. Trusted local connections and already accepted work continue. A Turn retains its original execution clients/tools/prompt; removal of referenced resources blocks the next preparation without consuming accepted input. Successful explicit Session configuration correction is the existing way to retry resource-missing input.

## Current limits

- This is production library composition, not the Host executable, CLI/config-file frontend, WSS listener startup, or completed `host.reload` transport route/operation receipt contract.
- Automatic compaction is not implemented by the current Session owner. This assembler explicitly rejects `execution.compaction.enabled = true`; callers must disable it rather than silently receive different behavior. Defaults currently enable compaction, so unmodified default Agent configuration is not a complete production setup.
- Workspace preparation uses the real configured-root backend and retains its canonical root in the Turn's prompt context. It does not install read/edit/shell tool modules; those remain a separate integration task.
- Model capabilities are explicit configuration declarations. Lists do not invent values for other, unknown capabilities. Full ModelRef strings and actual ToolSpec declarations are retained, without new model/tool IDs or a plugin platform.
- Historical incomplete-tool projection is being integrated separately. No history rewrite or invented tool result is performed here.
- A failed persistent configuration save remains an explicit blocker, with the original candidate retained. There is no new public repair command in this stage.

Tests use real Host/Runtime and loopback production HTTP/SSE with scripted replies, plus explicitly named scheduling gates and a harmless tool fixture. They are not live-provider or TUI validation.
