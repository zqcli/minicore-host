# minicore-host-provider

A real HTTP/SSE implementation of `minicore_runtime::model::Model` for
OpenAI-compatible **Chat Completions**. No endpoint is called by construction.
No default model, token cap, automatic retry layer or fake completion exists.

```rust,no_run
use std::{collections::BTreeSet, sync::Arc};
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel};
use minicore_runtime::model::{Model, ModelDescriptor, ReasoningPreference};

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

Bind that `Arc<dyn Model>` in the Host resource resolver and Runtime
ExecutionConfig. The secret must remain Host-private; do not place it in
Session defaults, public descriptors, RPC results or query URLs. No credential
is serialized by this crate. Diagnostics and Debug omit endpoint/body/secrets.

The supplied endpoint is exact. HTTPS certificate validation remains enabled.
Only numeric loopback IP addresses can use HTTP for local fixtures. Redirects
are refused, preventing bearer forwarding. System proxy configuration follows
reqwest; no proxy/TLS weakening is performed by this adapter.

Supported: one streamed choice (`n=1`), text, `reasoning_content` / `reasoning`,
refusal text, parallel function calls with original provider IDs, assistant/tool
history, explicit reasoning effort, request-supplied output token limit, usage
and finish metadata. Usage categories are normalized to Runtime's disjoint
input/output/reasoning/cache counts; missing details remain unknown. Raw
provider numeric counts and the exact finish reason are retained in bounded replay
metadata, including unknown future finish reasons.

The adapter requires both a provider `finish_reason` and SSE `[DONE]`. EOF,
malformed frames, incomplete tool arguments, mismatched indices and unsupported
content fail explicitly. It never synthesizes empty success or missing tool
arguments. Fully received frames produce partial progress before later failure. Tool
ID/name fragments that are not yet a valid Runtime ToolCallStart are retained
in ModelPartial.provider_replay under `minicore-host.openai-chat.partial.v1` on
failure, never made executable. An unknown finish cannot authorize tools.

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

## Validation baseline

The production manifest and the workspace's sole `Cargo.lock` pin published Runtime
`3aa47629929906832dcb309afb7d242c1ae3e53f` (`0.10.0`). It provides authoritative history/partial
reporting, retains complete Unknown responses before failing them, and reports
actual per-request retry-wait elapsed time. The earlier
isolated-crate checks used a local handoff patch; that is historical provenance,
not a current build prerequisite. No local Runtime patch is required now.

Task 010's historical focused provider checkpoint passed 5 unit tests, 31 HTTP/SSE
integration tests and 1 compiling doctest with its stated Runtime source. Its streamed
rejection regressions first failed against Host `6d29cdc`, despite the earlier
workspace gate passing. They exercise real loopback TCP requests and Runtime,
including measured retry lower bounds and controlled retry-wait cancellation
and deadlines. These focused checks are not a whole-foundation review pass or
live external provider validation. Run checks from the workspace root; the
obsolete isolated provider lock has been removed.

## Explicit remaining scope

- Responses, Anthropic, Gemini and other provider-specific adapters.
- Images, audio, video, legacy `function_call` and opaque encrypted/signature
  reasoning history. Unsupported data is rejected rather than silently lost.
- Provider-specific compatibility options and non-Bearer authentication.
- Live authorized endpoint validation and unchanged TUI integration. Loopback
  fixtures validate real transport and Runtime, not live model quality/access.

See `PROVENANCE.md`, `../../tasks/004-provider/TASK.md` and
`../../tasks/010-provider-retry/TASK.md` for source, regression and test provenance.
