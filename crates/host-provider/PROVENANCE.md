# Source and licensing provenance

This crate implements a new OpenAI-compatible Chat Completions adapter against
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
