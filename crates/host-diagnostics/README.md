# MiniCore process diagnostics

This crate implements the shared diagnostic pipeline from SPEC v4.93 chapter 12.
It is not business instrumentation, an event subscription, business persistence,
a log query service, or `host.info` wiring. Host/Agent/Session constructors do not
install it. Runtime and TUI are unchanged.

## Application ownership

- The entrypoint maps its existing `LogSettings` to `Options`: `tracing::Level`
  and either `Output::Stderr` or `Output::File(path)`. `Options::new` defaults to
  INFO and stderr. The application supplies its service name and optional version.
  There is no environment-variable override, rotation configuration, or fallback.
- `install(options)` installs the global tracing subscriber once and returns the
  unique `Guard`. Already-installed tracing, invalid service, file-open failure,
  and worker-start failure are explicit `InitError`s. Error summaries contain
  the operation/error kind, not a raw configured path or source error text.
- `prepare(options)` creates the same pipeline without global installation for
  embedding applications and isolated tests. It does not merge with an existing
  subscriber. The application still owns exactly one process pipeline.
- Cloned `Health` handles are read-only process-wide snapshots. Dropping a Host's
  handle cannot close another Host's writer. No full Logger is passed down owners.
- After business saves and cleanup, the entrypoint calls
  `guard.shutdown(timeout)`. It returns `Flushed`, `Failed`, or `TimedOut`.
  Dropping a forgotten guard waits at most one second. No shutdown branch prints
  a fallback message to stdout or stderr.

## Source contract

`EventSpec` declares a level, scope, stable component/event names, static safe
summary, and an explicit list of allowed attribute names. `emit` checks the
subscriber before evaluating the data closure. `EventData` accepts only actual
owned `Context`, an existing `Fault`, and scalar/scalar-array metadata. Unknown
attribute names are discarded before serialization. Unsupported third-party
tracing callsites are excluded, including arbitrary `Debug`, `Display`, and error
chains. A third-party integration must explicitly adapt safe metadata through
this API; enabling DEBUG or TRACE does not opt raw payloads in.

The source must still choose safe, event-appropriate attribute values. This is
not a content scanner or a regular-expression redactor. Do not declare contracts
for prompts, configuration, request/model/tool bodies, paths, authorization,
credentials, secret-bearing URLs, or raw exceptions. Static summaries and typed
scalar data do not make an arbitrary supplied string safe.

`EventData::fault(&fault)` preserves the original type/code pair and optional
diagnostic ID. It does not copy the Fault message/details or infer a different
fault from scope. Optional fields are omitted; unknown durations, counts,
completion, and persistence facts are not filled with zero or invented values.
Sources compute durations from a monotonic clock, not by subtracting UTC timestamps.

`Context` reuses protocol ID types and owns the snapshot, never the entities.
`context.instrument(future)` enters it on each poll. Spawned futures must
explicitly carry their context; do not retain `Span::enter` guards across await.
An explicit context replaces the complete parent chain, including clearing it
with an empty context. Independent child execution must supply its own real
chain; any origin relationship belongs in declared `origin_*` attributes.
Context spans survive event-level filtering and are short-lived operation spans,
not entity-lifetime performance spans.

The formatter emits JSONL with actual source-side UTC RFC3339 time and the shared
schema. Identity fields appear once, without a generic current-span/span-list
copy. Source metadata is serialized on the caller thread into a typed tracing
payload and then into the output record; async output removes writer I/O, not
those formatting/allocation costs. There is no separate text formatter.

## Output and health limits

The private writer uses one standard-library thread and unbounded channel.
It handles stderr or an append-only file at exactly the requested path. No
parent directories, permission changes, cross-process locks, quotas, retention,
rotation, or second emergency channel are added. File coordination belongs to
the application: unrelated processes must not share the same file. Applications
needing rotation can use externally managed stderr in this initial scope.

Only serialized records cross the writer boundary. Enqueue holds a short mutex
that never covers output I/O. The worker performs `write_all` for each record and
one final `flush`. Normal file/stderr outputs are not wrapped in another buffered
writer. Every failed write/flush attempt increments `write_failures`; a rejected
record increments `dropped_records`. These are observed process counters, not
claims that every underlying loss is measurable. Health reads do not log and do
not depend on the output destination. After an incomplete shutdown, the tail's
loss is unknown, so `dropped_records` is omitted and health is degraded.

A successful flush is not fsync or business persistence. I/O failures can leave
partial output; records are not retried or silently repaired. A stuck OS writer
can outlive the bounded shutdown wait, and process exit can lose the tail. The
unbounded channel has no product quota and can grow while output is stalled.
Crashes, aborts, and external termination do not synthesize clean shutdown facts.
There is no ERROR-delivery guarantee. These limits never authorize skipping
business history saves or blocking owner reclamation on diagnostic completion.

## Reference review

- Agent `f36a87366d8e2fe0a1c8216463887902e471c739`, `src/cli.rs`:
  reused the tracing/subscriber registry, application-entry ownership, default
  INFO intent, and stderr separation as code references. Host requires explicit
  initialization errors rather than Agent's ignored `try_init` error, JSONL
  rather than compact text, and explicit event metadata rather than arbitrary
  target-prefix capture. Agent is not the compatibility baseline.
- Agent `src/models/openai/http_error.rs`, `log_provider_request_failure`, and
  its log-redaction tests informed selective metadata and real tempfile tests;
  provider-error/request objects are not copied into this module.
- Inspected the actual downloaded `tracing-appender =0.2.5` source.
  `src/non_blocking.rs`, `WorkerGuard::drop`, calls `println!` when shutdown send
  times out; `src/worker.rs` echoes a raw final flush error to stderr and does not
  expose I/O-failure counts. Its guard is not used or wrapped. The small private
  writer follows only the mature separation of producer, writer, and exit guard;
  its stop/completion path and observed counters are implemented here. No
  process-wide stdio redirection or leaked appender guard hides that defect.

Tests exercise real tempfile append/output, actual source identities, source
filtering, scalar values, ordinary write/flush errors, and blocked writer exit.
The subprocess checks assert the exact known libtest startup banner followed by
only a test RPC line on stdout; normal stderr is parsed as JSONL, and error and
both explicit/Drop timeout branches produce no extra stderr. These are module
contract tests, not live Host/Runtime/TUI integration results.
