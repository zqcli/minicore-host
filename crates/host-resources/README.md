# Production Host resource assembly

`HostResourceBuilder` composes real OpenAI-compatible HTTP/SSE clients, the statically compiled tool registry, canonical workspace bindings, execution deadlines/retries, and frozen Host/Role/Agent prompt layers per Turn. The built-in registry contains `agent`, `read`, `write`, `edit`, `apply_patch`, and `bash`; registration alone does not enable them. HostConfig.tools selects available definitions, and SessionConfig.tools selects the actual ToolSet.

The source follows those existing preparation boundaries:

| Module | Responsibility |
| --- | --- |
| `builder.rs` | Private credential inputs, immutable Host candidates, provider construction, and builder-owned shared bindings across reloads |
| `resolver.rs` | Session validation, frozen executable Turn bindings, output wrapping, and separate declaration-only compaction resources |
| `prompt.rs` | Frozen Host/Role/Agent/workspace context combined with the existing core history and provenance preparation |
| `workspaces.rs` | Weak sharing of actual canonical Workspace mutation bindings across overlapping Turns |
| `approval.rs` | The selected execution policy and the actual built-in read/agent exemptions |

Host candidate selection and compaction use the registry's exact ToolSpec lookup.
Each caller keeps its own existing error contract: invalid Host configuration
is distinct from missing Session resources. Shared prompt construction does not
bind executable tools or create activation authority for compaction.

Each configured model requires an explicit `api`: `chat_completions` or
`responses`. The builder constructs that exact provider without endpoint
guessing, network probes or fallback. Model discovery includes the selected
protocol without private endpoint, provider-model or credential-source fields.
Ordinary and compaction bindings select the same configured provider. Existing
Turns retain their bound protocol and credentials across a configuration reload.

Keep one builder (or its clones) across reloads, so the agent tool's independently owned delegation scopes and concurrent Turns' actual Workspace mutation binding remain shared. `prepare` reads only explicitly named environment variables. `prepare_with_credentials` supports explicit private embedding inputs, including isolated fixture credentials. Neither path sends model requests or executes tools to test availability.

A directory-based production setup passes one configuration directory and builder to the Host startup entry. Host loads native JSON through `minicore-host-config`, then prepares the complete candidate with this builder before publishing it. Directory declarations supply current Host and managed Agent settings; durable records retain their identities and existing Sessions. The programmatic `Host::memory_configured` and `Host::open_configured` entries remain available for embedding. Provider and network key bytes are never serialized. There is one `HostRecord<HostConfig>` disk shape; the name-only HostSettings convenience does not define another storage format.

External `host.reload` accepts an operation ID and expected configuration revision, then reloads the Host's bound directory. Clients do not parse or submit Host candidates. Programmatic callers may still prepare a complete candidate without owner locks and use `Host::reload_configuration`. A changed listener address or any logging setting returns RestartRequired before Host or managed Agent changes. Unchanged startup settings do not block a hot update. The application owns installing its logging backend exactly once and creating its listener; this crate does not do either.

After acceptance, Host owns the original configuration save and publication independently of the waiter's connection/future. Saves run outside owner locks. Version reservation excludes a competing write before it reaches disk. Failed saves preserve the live snapshot/key, retain the candidate and fault, and prevent a false successful shutdown. No new save-recovery interface, automatic retry, rollback, external-file snapshot protocol, or config migration is added.

Successful publication replaces one immutable resource snapshot and changes network authentication at the same boundary. Only actual network key changes revoke old network connections. Trusted local connections and already accepted work continue. A Turn retains its original execution clients/tools/prompt; removal of referenced resources blocks the next preparation without consuming accepted input. Successful explicit Session configuration correction is the existing way to retry resource-missing input.

## Tool policy and credentials

`SessionConfig.execution.approval` is frozen per Turn: omitted/`ask` requests
approval for each mutating call, `auto` permits it without that interaction, and
`read_only` rejects it. Only the actual builtin `read` and builtin `agent`
inspect/wait actions are read-only exemptions; an external factory with the same
name does not acquire them. Approval uses the existing Runtime interaction and
`interaction.answer`, with no remembered allow-once decision. See
[`approval.rs`](src/approval.rs) and the [actual-binary tests](../host-app/tests/tool_approval.rs).

Production command bindings exclude configured model/network credential source
names at each spawn. Successful publications accumulate names for the Host
lifetime, including for later commands in an already-bound Turn. Failed
candidates do not change that set. This does not alter the parent environment,
already-started children or tool output, and is not filesystem/process isolation.
The [executable regression](../host-app/tests/credential_environment.rs) checks
that boundary; standalone local-tool callers choose their own environment policy.

## Current limits

- Executable/CLI/config-file and listener startup live in `host-app`; canonical `host.reload` receipts and publication live in `host-core` and route through `host-transport`. This crate prepares resources, not those entrypoints.
- The Session owner supports automatic compaction before Session model requests using the original bound model/settings. Manual compaction prepares model, reasoning, selected ToolSpecs and the Host/Role/Agent/workspace prompt without creating a Turn or binding tools. Enabling compaction is supported; actual utility calls and their persistence can still fail and report their own facts.
- Local tools bind the actual selected implementations to the original prepared Turn, Session output/edit owner, and canonical Workspace. Old and new resource publications share a builder-owned weak Workspace map, so overlapping Turns on the same root share the existing mutation guard without extending unused roots' lifetime. Arbitrary shell scripts and outside processes are not coordinated by that guard.
- Normal Turn preparation uses that shared Workspace binding; manual compaction continues to bind its prompt's canonical workspace directly. It has no executable tool mutation work to register, and this source split does not change either binding path.
- Public tool Finished state and completion wait for the actual per-call file/process cleanup, with final output positions. Native results enter history immediately; normal completed tools and argument rejection do not wait for Turn completion. The existing Turn collector still owns actual worker joins and blocks finalization on unconfirmed cleanup.
- Actual edits enter the Session-owned EditJournal; process text enters the existing output store. Native ToolResults still use existing history save barriers. Edit snapshots and process observations survive unload but not source reclamation or restart. `host-core` and `host-transport` expose the existing `workspace.*`/`changes.*` queries; there is no permanent edit database.
- Public output contains unchanged UTF-8 prefixes, including code points split across actual pipe chunks. Binary or incomplete EOF bytes are never replaced; text reads crossing that prefix explicitly fail. Successful native CommandResult preserves all raw bytes. Empty observed stdout/stderr is distinct from a missing source.
- Model capabilities are explicit configuration declarations. Lists do not invent values for other, unknown capabilities. Full ModelRef strings and actual ToolSpec declarations are retained, without new model/tool IDs or a plugin platform.
- Historical incomplete-tool projection preserves complete native exchanges and explains genuinely missing old results in the model prompt. Compaction does not rewrite original history or invent tool results.
- A failed persistent configuration save remains an explicit blocker, with the original candidate retained. There is no new public repair command in this stage.

Tests use real Host/Runtime, actual temporary files/processes, and loopback production HTTP/SSE with scripted replies, plus explicitly named scheduling gates and a harmless tool fixture for the earlier custom-registry tests. They are not live-provider or TUI validation.

`tests/support/production.rs` supplies the shared production fixture for local
tools, input provenance, approvals, credential environments, and builder
concurrency tests. `tests/local_tools.rs` and its completion/process modules
retain the actual worker cleanup and cancellation witnesses.
`tests/input_provenance.rs` and its input/compaction modules retain the original
input identity, queue, history, and compaction coverage. The private builder
concurrency test still holds a real Workspace mutation guard as its gate.

Host resource preparation validates and canonicalizes `workspace_roots` into the
immutable configuration candidate. The existing Host publication boundary applies
new roots together; preparing or rejecting a candidate changes no directory scope.
