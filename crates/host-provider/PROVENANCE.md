# Source and licensing provenance

This crate implements OpenAI-compatible Chat Completions and Responses adapters against
MiniCore Runtime's actual Model trait. It does not embed or emulate the Agent
or Runtime owner layers.

The transport, SSE framing, request-history projection, error delivery-state
classification, and usage normalization were informed by these source files in
`zqcli/minicore-agent` at commit
`f36a87366d8e2fe0a1c8216463887902e471c739`:

- `src/models/openai.rs`
- `src/models/openai/{request,sse,http_error,errors,terminal}.rs`

The pinned upstream implements the **Responses API**, not Chat Completions.
Chat request encoding, choice/tool delta handling, and strict finish plus DONE
handling here are new. The CR/LF/multiline SSE framing and conservative error
classification patterns are selectively adapted, not a wholesale module copy.
The upstream package declares `MIT OR Apache-2.0` in Cargo.toml; this crate uses
the same dual license. That checkout has no separate LICENSE or NOTICE file.
Original upstream copyrights and the choice of upstream license remain intact.

The Host 0.11.0 Responses implementation also consulted that same pinned Agent
checkout's `src/models/openai/{request,reasoning,replay,terminal,tools}.rs`.
It reuses the established request shapes, distinctions between provisional and
completed reasoning items, native item/call identity, and disjoint usage mapping.
The Host implementation uses its existing safe transport, framer, classifier and
Runtime boundary. It does not import the Agent's event quotas, full replay/body
buffers, duplicate argument parsing, permissive unknown events or suffix-based
completion reconciliation. Native replay stores ordinary text ranges and tool
identities; completed arguments remain exclusively Runtime-owned.

Responses wire fields and lifecycle were checked against the official OpenAI
API documentation on 2026-11-13:

- <https://developers.openai.com/api/reference/resources/responses/methods/create>
- <https://developers.openai.com/api/reference/resources/responses/streaming-events>

These establish stateless encrypted-reasoning inclusion, assistant `phase`,
separate output item IDs and function `call_id`, the completed reasoning item as
the replay source, and distinct completed/failed/incomplete terminal events.
Tests use explicitly scripted loopback HTTP/SSE replies, not live service output.

Runtime is pinned to the real source revision recorded in Cargo.toml. Local
path patches used during integration are listed in the task log and are not
substitute Runtime implementations.

Behavior intentionally stricter than the old Agent adapter:

- Unknown 429/5xx reasons are not treated as proven retryable rejections.
- reqwest protocol retries and redirects are explicitly disabled.
- HTTPS is required except literal loopback HTTP. URL credentials, query and
  fragment are rejected. API keys are accepted only through a nonserializable
  secret argument or the named environment variable.
- Provider error free-text and reqwest error strings are not retained. Safe
  diagnostics retain HTTP status and a structured ModelError classification.
- Streamed text/reasoning, accumulated tool arguments and HTTP error bodies
  have no adapter byte quota. Provider deltas are forwarded without artificial
  event-size splitting. Runtime still validates ToolCallId/ToolName grammar and
  JSON parser nesting safety; malformed streams never fabricate completion.
- HTTP error classification consumes the complete body under the original
  cancellation/deadline. Only recognized structured error codes, status and
  response retry headers may authorize Runtime retries; provider free-text is
  still excluded from diagnostics.
