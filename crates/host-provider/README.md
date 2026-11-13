# minicore-host-provider

A real HTTP/SSE implementation of `minicore_host_runtime::model::Model` for
OpenAI-compatible **Chat Completions** and **Responses**. No endpoint is called by construction.
No default model, token cap, automatic retry layer or fake completion exists.

```rust,no_run
use std::{collections::BTreeSet, sync::Arc};
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel};
use minicore_host_runtime::model::{Model, ModelDescriptor, ReasoningPreference};

let descriptor = ModelDescriptor::new(
    "production/chat".parse()?,
    128_000, // Set the actual configured provider/model capability.
    BTreeSet::from([ReasoningPreference::Auto]),
    true,
)?;
let config = OpenAiChatConfig::new(
    "https://api.openai.com/v1/chat/completions",
    "your-authorized-model",
    descriptor,
);
let model: Arc<dyn Model> = Arc::new(OpenAiChatModel::from_env(config, "OPENAI_API_KEY")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

For an explicitly configured Responses endpoint, use its own client:

```rust,no_run
use std::collections::BTreeSet;
use minicore_host_provider::{OpenAiResponsesConfig, OpenAiResponsesModel};
use minicore_host_runtime::model::{ModelDescriptor, ReasoningPreference};

let descriptor = ModelDescriptor::new(
    "production/responses".parse()?,
    128_000, // Set the actual configured provider/model capability.
    BTreeSet::from([ReasoningPreference::Auto, ReasoningPreference::High]),
    true,
)?;
let config = OpenAiResponsesConfig::new(
    "https://api.openai.com/v1/responses",
    "your-authorized-model",
    descriptor,
);
let model = OpenAiResponsesModel::from_env(config, "OPENAI_API_KEY")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Bind that `Arc<dyn Model>` in the Host resource resolver and Runtime
ExecutionConfig. The secret must remain Host-private; do not place it in
Session defaults, public descriptors, RPC results or query URLs. No credential
is serialized by this crate. Diagnostics and Debug omit endpoint/body/secrets.

The supplied endpoint is exact. HTTPS certificate validation remains enabled.
Only numeric loopback IP addresses can use HTTP for local fixtures. Redirects
are refused, preventing bearer forwarding. System proxy configuration follows
reqwest; no proxy/TLS weakening is performed by this adapter.

Chat Completions supports one streamed choice (`n=1`), text, `reasoning_content` / `reasoning`,
refusal text, parallel function calls with original provider IDs, assistant/tool
history, explicit reasoning effort, request-supplied output token limit, usage
and finish metadata. Usage categories are normalized to Runtime's disjoint
input/output/reasoning/cache counts; missing details remain unknown. Raw
provider numeric counts and the exact finish reason are retained in replay
metadata, including unknown future finish reasons.

Ordinary frames decode directly to typed provider fields. A separate error
envelope is read only when normal decoding fails, so a structured rejection
still takes precedence over malformed normal fields. Duplicate recognized
typed-message and error-envelope fields are rejected rather than repaired
through a generic JSON object; error payload classification keeps its existing rules.
The whole frame must be valid UTF-8. Unused extension fields follow serde's
skip behavior; fields used by the adapter keep their typed validation.

Chat Completions requires both a provider `finish_reason` and SSE `[DONE]`. EOF,
malformed frames, mismatched indices and unsupported content fail explicitly.
Runtime alone accumulates raw tool arguments and parses their JSON object at
tool end; it does not repair control characters or incomplete JSON. The adapter
never retains a second cumulative argument buffer after a tool starts, and never
synthesizes empty success or missing arguments. Fully received frames produce
partial progress before later failure. Tool
ID/name fragments that are not yet a valid Runtime ToolCallStart are retained
in ModelPartial.provider_replay under `minicore-host.openai-chat.partial.v1` on
failure, never made executable. An unknown finish cannot authorize tools.

At the provider's valid protocol endpoint, finish/usage replay metadata is
delivered before tool ends. Its `minicore-host.openai-chat.v1` format can therefore
appear in a failed ModelPartial when Runtime rejects arguments; the format is
not a successful-response marker. If one tool validates before a later tool
fails, the partial retains that verified call and the remaining raw arguments.
The whole model call still fails, and none of its tools may execute. Only the
ModelDriver result distinguishes a complete response from a partial.

Runtime alone retries a model call. The adapter sends exactly one request per
`start`; only verified connection-before-send failure or recognized pre-start
temporary rejection has a retry hint. Role-only output already counts as
Started. Cancellation/deadline while sending have Unknown delivery; cancellation,
timeout or errors after provider events have Started delivery and never retry.
Recognized pre-event SSE rejections preserve the response HTTP status and the
same parsed `Retry-After` / `Retry-After-ms` lower bound as HTTP error responses.
The absolute hint is captured when headers arrive, so parsing the body does not
restart its clock. Runtime stops if the remaining wait exceeds its 30-second
limit or does not fit the original deadline; the adapter never clamps the hint,
waits or resends. A header does not make Unknown, Started, quota or authentication
failures retryable.

## Responses

`OpenAiResponsesModel` shares the same HTTP client policy, SSE framing, error
classification, diagnostics, cancellation and absolute deadlines. It sends one
stateless request per `start`: full `input`, `stream: true`, `store: false`,
`truncation: disabled`, and `include: ["reasoning.encrypted_content"]`. It never
uses `previous_response_id` or a provider-side conversation. An output limit is
sent only when supplied by Runtime. Declared function schemas are forwarded
without silently enabling strict schema conversion. Configured reasoning effort
is forwarded; reasoning-capable descriptors also request an automatic summary.

Supported output consists of assistant text/refusal messages, reasoning text and
summary parts, and function calls. The adapter checks event sequence, output and
content indices, item identities, call IDs, lifecycle transitions and repeated
completion snapshots. It retains only byte counts and streaming SHA-256 evidence
for handed-off text and arguments. It does not keep a second cumulative body or
parse a completed argument object. A disagreeing snapshot fails rather than
appending a suffix or replacing already observed output.

Only a valid `response.completed` terminal can complete the protocol. A failed
or incomplete response, error event, `[DONE]` without a Responses completion,
malformed event, interruption or EOF returns a failure with actual partial data.
`response.created` already establishes Started delivery, even before visible
text. Native completion/usage metadata is emitted before tool ends; Runtime
alone validates arguments and returns the authoritative complete/partial result.
No tool from a failed model request becomes executable.

Completed history uses `minicore-host.openai-responses.v1` replay metadata. Native
reasoning IDs, summaries and encrypted content come from `output_item.done`;
provisional encrypted content in `output_item.added` is never replayed. Original
function item IDs and call IDs, item order and assistant `phase` are preserved.
Message content is reconstructed from ranges in Runtime's canonical normalized
text; function arguments come from Runtime's validated ToolCall. Ordinary text
and argument bodies are not duplicated in replay metadata. Native metadata stays
opaque to Runtime and is excluded from diagnostics and public progress.

The completed format may also appear in a ModelPartial after Runtime rejects an
argument at tool end; its format tag never proves success. Earlier failures use
`minicore-host.openai-responses.partial.v1`, preserving completed native reasoning,
received status/usage, and an invalid tool header where applicable. Partial replay
is not accepted as complete input. Foreign protocol/model replay, inconsistent
canonical content, and standalone reasoning without a native item identity are
rejected locally rather than silently omitted or fabricated.

## Validation baseline

The production manifest uses the internal `host-runtime` workspace package.
Its shared `ModelDriver` returns authoritative complete/partial responses and
actual per-request retry-wait elapsed time. A complete Unknown response retains
its original finish reason; the Session accepts that response before ending the
Turn with the corresponding failure. Provider tests call the real driver and
explicit tool batches, without a standalone AgentLoop lifecycle. Imported source
and integrated validation are recorded in Task 106.

Task 010's historical focused provider checkpoint passed 5 unit tests, 31 HTTP/SSE
integration tests and 1 compiling doctest with its stated Runtime source. Its streamed
rejection regressions first failed against Host `6d29cdc`, despite the earlier
workspace gate passing. They exercise real loopback TCP requests and Runtime,
including measured retry lower bounds and controlled retry-wait cancellation
and deadlines. These focused checks are not a whole-foundation review pass or
live external provider validation. Run checks from the workspace root; the
obsolete isolated provider lock has been removed.

## Explicit remaining scope

- Anthropic, Gemini and other provider-specific adapters.
- Images, audio, video, legacy `function_call` and opaque encrypted/signature
  reasoning history outside the native Responses format. Other Responses item
  types and events are rejected explicitly; unused extension fields are skipped.
- Provider-specific compatibility options and non-Bearer authentication.
- Live authorized endpoint validation and unchanged TUI integration. Loopback
  fixtures validate real transport and Runtime, not live model quality/access.

See `PROVENANCE.md`, `../../tasks/004-provider/TASK.md` and
`../../tasks/010-provider-retry/TASK.md` for source, regression and test provenance.
The Host 0.11.0 Responses work is recorded in
`../../tasks/109-host-capabilities/responses.md`.
