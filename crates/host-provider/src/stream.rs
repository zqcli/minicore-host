use std::collections::VecDeque;

use minicore_host_runtime::{
    ToolCallId,
    model::{ModelError, ModelErrorKind, ModelEvent, ModelFinishReason, ProviderReplay, Usage},
    tools::ToolName,
};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::{error, sse::Frames};

pub(crate) const REPLAY_FORMAT: &str = "minicore-host.openai-chat.v1";
const PARTIAL_REPLAY_FORMAT: &str = "minicore-host.openai-chat.partial.v1";

pub(crate) struct State {
    frames: Frames,
    error_context: error::ResponseContext,
    pending: VecDeque<ModelEvent>,
    pending_error: Option<ModelError>,
    cancellation: CancellationToken,
    deadline: Instant,
    done: bool,
    seen: bool,
    has_output: bool,
    tools: Vec<Tool>,
    finish: Option<String>,
    usage: Option<Usage>,
    raw_usage: Option<Value>,
    response_id: Option<String>,
}

struct Tool {
    id: String,
    name: String,
    // Only retained when the first argument fragment cannot form a valid start.
    // Started calls hand every fragment directly to Runtime's sole assembler.
    arguments: Option<String>,
    started: bool,
}

impl State {
    pub(crate) fn new(
        response: reqwest::Response,
        cancellation: CancellationToken,
        deadline: Instant,
        authorization: &reqwest::header::HeaderValue,
    ) -> Self {
        // Retain response metadata before bytes_stream consumes the response.
        // An explicit first-frame rejection uses the same header lower bound
        // as an HTTP error. Runtime alone decides whether/how long to retry.
        let error_context = error::ResponseContext::new(&response, authorization);
        Self {
            frames: Frames::new(Box::pin(response.bytes_stream())),
            error_context,
            pending: VecDeque::new(),
            pending_error: None,
            cancellation,
            deadline,
            done: false,
            seen: false,
            has_output: false,
            tools: Vec::new(),
            finish: None,
            usage: None,
            raw_usage: None,
            response_id: None,
        }
    }

    fn malformed(&self) -> ModelError {
        error::stream(ModelErrorKind::InvalidProviderResponse, self.seen)
    }

    fn fail(&mut self, error: ModelError) -> Result<ModelEvent, ModelError> {
        let error = self.error_context.attach(error);
        self.pending.clear();
        self.done = true;
        // Runtime's typed ToolCallStart requires a valid, complete ID/name.
        // Preserve provider fragments that cannot form one in its existing
        // opaque partial metadata, never by inventing an executable call.
        let headers: Vec<_> = self.tools.iter().enumerate()
            .filter(|(_, tool)| !tool.started && (!tool.id.is_empty() || !tool.name.is_empty() || tool.arguments.is_some()))
            .map(|(index, tool)| json!({"index":index, "id_fragment":tool.id, "name_fragment":tool.name, "arguments_fragment":tool.arguments.as_deref().unwrap_or_default()}))
            .collect();
        if headers.is_empty() && self.finish.is_none() && self.raw_usage.is_none() {
            return Err(error);
        }
        let mut metadata = json!({});
        if !headers.is_empty() {
            metadata["incomplete_tool_headers"] = json!(headers);
        }
        if let Some(reason) = &self.finish {
            metadata["finish_reason"] = json!(reason);
        }
        if let Some(usage) = &self.raw_usage {
            metadata["usage"] = usage.clone();
        }
        match ProviderReplay::new(PARTIAL_REPLAY_FORMAT, metadata) {
            Ok(replay) => {
                self.pending_error = Some(error);
                Ok(ModelEvent::ProviderReplay { replay })
            }
            // Keep Runtime's JSON nesting and replay-format validation.
            Err(_) => Err(error),
        }
    }

    fn handle(&mut self, frame: &[u8]) -> Result<(), ModelError> {
        if frame == b"[DONE]" {
            return self.complete();
        }
        // Ignored extension fields skip string decoding. Validate the entire
        // SSE payload's UTF-8 before the typed decoder skips those fields.
        let frame = std::str::from_utf8(frame).map_err(|_| self.malformed())?;
        let chunk = match serde_json::from_str::<Chunk>(frame) {
            Ok(chunk) => chunk,
            Err(_) => {
                // A structured rejection takes precedence over malformed normal
                // fields. Decode only its envelope on this exceptional path;
                // never retry the normal chunk through a permissive Value tree.
                let envelope: ErrorEnvelope =
                    serde_json::from_str(frame).map_err(|_| self.malformed())?;
                self.error_context
                    .record_request_id(envelope.request_id.as_ref().and_then(Value::as_str));
                return Err(match envelope.error {
                    Some(value) => error::provider(&value, self.seen, &self.error_context),
                    None => self.malformed(),
                });
            }
        };
        self.error_context
            .record_request_id(chunk.request_id.as_ref().and_then(Value::as_str));
        if let Some(value) = chunk.error {
            return Err(error::provider(&value, self.seen, &self.error_context));
        }
        // Receiving a provider choice, including its role-only header, proves
        // the generation has started even before text reaches Runtime.
        self.seen = true;
        if let Some(id) = chunk.id {
            if id.is_empty()
                || self
                    .response_id
                    .as_ref()
                    .is_some_and(|previous| previous != &id)
            {
                return Err(self.malformed());
            }
            self.response_id = Some(id);
        }
        let reported_usage = chunk.usage.is_some();
        if let Some(usage) = chunk.usage {
            self.raw_usage = Some(serde_json::to_value(&usage).map_err(|_| self.malformed())?);
            let usage = usage.runtime().map_err(|_| self.malformed())?;
            if self.usage.replace(usage).is_some() {
                return Err(self.malformed());
            }
            self.pending.push_back(ModelEvent::Usage { usage });
        }
        if chunk.choices.is_empty() {
            // OpenAI's usage trailer is the only empty-choice data frame.
            if self.finish.is_none() || !reported_usage {
                return Err(self.malformed());
            }
            return Ok(());
        }
        if self.finish.is_some() || chunk.choices.len() != 1 {
            return Err(self.malformed());
        }
        let choice = chunk.choices.into_iter().next().expect("one choice");
        if choice.index != 0
            || choice
                .delta
                .role
                .as_deref()
                .is_some_and(|role| role != "assistant")
        {
            return Err(self.malformed());
        }
        if choice.delta.reasoning.is_some() && choice.delta.reasoning_content.is_some() {
            return Err(self.malformed());
        }
        if let Some(reasoning) = choice.delta.reasoning_content.or(choice.delta.reasoning) {
            self.queue_text(&reasoning, true);
        }
        if let Some(text) = choice.delta.content {
            self.queue_text(&text, false);
        }
        if let Some(refusal) = choice.delta.refusal {
            self.queue_text(&refusal, false);
        }
        if let Some(tools) = choice.delta.tool_calls {
            for tool in tools {
                self.tool_delta(tool)?;
            }
        }
        if let Some(finish) = choice.finish_reason {
            if finish.is_empty() || finish.chars().any(char::is_control) {
                return Err(self.malformed());
            }
            self.finish = Some(finish);
        }
        Ok(())
    }

    fn queue_text(&mut self, value: &str, reasoning: bool) {
        self.has_output |= !value.is_empty();
        if !value.is_empty() {
            self.pending.push_back(if reasoning {
                ModelEvent::reasoning_delta(value)
            } else {
                ModelEvent::text_delta(value)
            });
        }
    }

    fn tool_delta(&mut self, delta: ToolDelta) -> Result<(), ModelError> {
        let index = usize::try_from(delta.index).map_err(|_| self.malformed())?;
        if index > self.tools.len() || delta.kind.as_deref().is_some_and(|kind| kind != "function")
        {
            return Err(self.malformed());
        }
        if index == self.tools.len() {
            self.tools.push(Tool {
                id: String::new(),
                name: String::new(),
                arguments: None,
                started: false,
            });
        }
        let tool = &mut self.tools[index];
        let function = delta.function.unwrap_or_default();
        let mut arguments = function.arguments;
        if !tool.started {
            tool.arguments = arguments.take();
        }
        if let Some(id) = delta.id {
            if tool.started {
                if id != tool.id {
                    return Err(self.malformed());
                }
            } else {
                tool.id.push_str(&id);
            }
        }
        if let Some(name) = function.name {
            if tool.started {
                if name != tool.name {
                    return Err(self.malformed());
                }
            } else {
                tool.name.push_str(&name);
            }
        }
        // These ceilings are Runtime's actual structural ToolCallId/ToolName
        // grammar, not Host execution quotas. Never truncate provider values.
        if tool.id.len() > 256 || tool.name.len() > 64 {
            return Err(self.malformed());
        }
        if self.tools.iter().enumerate().any(|(other, tool)| {
            other != index && tool.id == self.tools[index].id && !tool.id.is_empty()
        }) {
            return Err(self.malformed());
        }
        // Runtime assigns call_index by ToolCallStart order. Do not silently
        // reorder a provider's incomplete metadata across parallel calls.
        if self.tools[index].arguments.is_some()
            && self.tools[..index].iter().any(|tool| !tool.started)
        {
            return Err(self.malformed());
        }
        let tool = &mut self.tools[index];
        if tool.arguments.is_some() {
            let id = ToolCallId::new(&tool.id)
                .map_err(|_| error::stream(ModelErrorKind::InvalidProviderResponse, true))?;
            let name: ToolName = tool
                .name
                .parse()
                .map_err(|_| error::stream(ModelErrorKind::InvalidProviderResponse, true))?;
            tool.started = true;
            self.has_output = true;
            self.pending.push_back(ModelEvent::ToolCallStart {
                tool_call_id: id,
                tool_name: name,
            });
            arguments = tool.arguments.take();
        }
        if let Some(arguments) = arguments {
            let id = ToolCallId::new(&tool.id)
                .map_err(|_| error::stream(ModelErrorKind::InvalidProviderResponse, true))?;
            self.pending
                .push_back(ModelEvent::tool_call_arguments_delta(id, arguments));
        }
        Ok(())
    }

    fn complete(&mut self) -> Result<(), ModelError> {
        if !self.has_output {
            return Err(self.malformed());
        }
        let raw = self
            .finish
            .as_deref()
            .ok_or_else(|| error::stream(ModelErrorKind::IncompleteResponse, self.seen))?;
        let reason = match raw {
            "stop" => ModelFinishReason::Stop,
            "tool_calls" => ModelFinishReason::ToolCalls,
            "length" => ModelFinishReason::Length,
            "content_filter" => ModelFinishReason::ContentFiltered,
            "refusal" | "refused" => ModelFinishReason::Refused,
            _ => ModelFinishReason::Unknown,
        };
        if (!self.tools.is_empty() && reason != ModelFinishReason::ToolCalls)
            || (self.tools.is_empty() && reason == ModelFinishReason::ToolCalls)
        {
            return Err(self.malformed());
        }
        if self.tools.iter().any(|tool| !tool.started) {
            return Err(self.malformed());
        }
        // Preserve the provider's exact finish reason, including future reasons
        // that map to Runtime's Unknown. No endpoint or secret belongs here.
        let mut metadata = json!({"finish_reason": raw});
        if let Some(usage) = self.raw_usage.take() {
            metadata["usage"] = usage;
        }
        let replay = ProviderReplay::new(REPLAY_FORMAT, metadata).map_err(|_| self.malformed())?;
        self.pending
            .push_back(ModelEvent::ProviderReplay { replay });
        // Protocol metadata is already known, even if Runtime rejects an
        // argument object at End. Only the Driver can confirm a full response.
        for tool in &self.tools {
            self.pending.push_back(ModelEvent::ToolCallEnd {
                tool_call_id: ToolCallId::new(&tool.id).map_err(|_| self.malformed())?,
            });
        }
        self.pending.push_back(ModelEvent::Finish { reason });
        self.done = true;
        Ok(())
    }
}

pub(crate) async fn next(mut state: State) -> Option<(Result<ModelEvent, ModelError>, State)> {
    loop {
        // Pending events belong to a single, already parsed frame. Return them
        // before processing another frame to preserve observed partial output.
        if let Some(event) = state.pending.pop_front() {
            return Some((Ok(event), state));
        }
        if let Some(error) = state.pending_error.take() {
            return Some((Err(error), state));
        }
        if state.done {
            return None;
        }
        let failure = if state.cancellation.is_cancelled() {
            Some(error::stream(ModelErrorKind::Cancelled, state.seen))
        } else if Instant::now() >= state.deadline {
            Some(error::stream(ModelErrorKind::Timeout, state.seen))
        } else {
            None
        };
        if let Some(error) = failure {
            let result = state.fail(error);
            return Some((result, state));
        }
        let frame = tokio::select! {
            biased;
            _ = state.cancellation.cancelled() => Err(ModelErrorKind::Cancelled),
            _ = tokio::time::sleep_until(state.deadline) => Err(ModelErrorKind::Timeout),
            frame = state.frames.frame() => frame,
        };
        let result = match frame {
            Ok(frame) => state.handle(&frame),
            Err(kind) => Err(error::stream(kind, state.seen)),
        };
        if let Err(error) = result {
            // Never fabricate Finish or partial tool end for an invalid frame.
            // Earlier valid frames remain observed.
            let result = state.fail(error);
            return Some((result, state));
        }
    }
}

#[derive(Deserialize)]
struct Chunk {
    request_id: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    id: Option<String>,
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<ProviderUsage>,
}
#[derive(Deserialize)]
struct ErrorEnvelope {
    request_id: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
    // These fields are irrelevant to a rejection, but repeated envelope fields
    // are still malformed. Their nested normal-response types are not decoded.
    #[serde(default, rename = "id")]
    _id: Option<IgnoredAny>,
    #[serde(default, rename = "choices")]
    _choices: Option<IgnoredAny>,
    #[serde(default, rename = "usage")]
    _usage: Option<IgnoredAny>,
}
#[derive(Deserialize)]
struct Choice {
    index: u32,
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Delta {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolDelta>>,
}
#[derive(Deserialize)]
struct ToolDelta {
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}
#[derive(Default, Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
#[derive(Deserialize, Serialize)]
struct ProviderUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_tokens_details: Option<PromptDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_tokens_details: Option<CompletionDetails>,
}
#[derive(Deserialize, Serialize)]
struct PromptDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_tokens: Option<u64>,
}
#[derive(Deserialize, Serialize)]
struct CompletionDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_tokens: Option<u64>,
}
impl ProviderUsage {
    fn runtime(self) -> Result<Usage, ()> {
        let cached = self
            .prompt_tokens_details
            .and_then(|details| details.cached_tokens);
        let reasoning = self
            .completion_tokens_details
            .and_then(|details| details.reasoning_tokens);
        // OpenAI totals include cached/reasoning tokens. Runtime's categories
        // are disjoint. Missing details stay unknown, never an invented zero.
        let input = match (self.prompt_tokens, cached) {
            (Some(total), Some(cached)) => Some(total.checked_sub(cached).ok_or(())?),
            _ => None,
        };
        let output = match (self.completion_tokens, reasoning) {
            (Some(total), Some(reasoning)) => Some(total.checked_sub(reasoning).ok_or(())?),
            _ => None,
        };
        if let (Some(input), Some(output), Some(total)) = (
            self.prompt_tokens,
            self.completion_tokens,
            self.total_tokens,
        ) && input.checked_add(output) != Some(total)
        {
            return Err(());
        }
        Ok(Usage::from_optional(input, output, reasoning)
            .with_cache_read_tokens(cached)
            .with_provider_total_tokens(self.total_tokens))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minicore_host_runtime::model::{DeliveryState, RetryHint};

    fn state(chunk: &'static [u8]) -> State {
        State {
            frames: Frames::new(Box::pin(futures_util::stream::iter([Ok(
                bytes::Bytes::from_static(chunk),
            )]))),
            error_context: error::ResponseContext::fixture(200),
            pending: VecDeque::new(),
            pending_error: None,
            cancellation: CancellationToken::new(),
            deadline: Instant::now() + std::time::Duration::from_secs(60),
            done: false,
            seen: false,
            has_output: false,
            tools: Vec::new(),
            finish: None,
            usage: None,
            raw_usage: None,
            response_id: None,
        }
    }

    #[tokio::test]
    async fn interruption_drains_parsed_frame_but_preempts_buffered_terminal_frame() {
        for kind in [ModelErrorKind::Cancelled, ModelErrorKind::Timeout] {
            let state = state(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"think\",\"content\":\"text\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
            let (event, mut state) = next(state).await.unwrap();
            assert!(matches!(event.unwrap(), ModelEvent::ReasoningDelta { .. }));
            if kind == ModelErrorKind::Cancelled {
                state.cancellation.cancel();
            } else {
                state.deadline = Instant::now();
            }
            let (event, state) = next(state).await.unwrap();
            assert!(matches!(event.unwrap(), ModelEvent::TextDelta { .. }));
            let (event, state) = next(state).await.unwrap();
            assert!(
                matches!(event.unwrap(), ModelEvent::ProviderReplay { replay }
                if replay.format() == PARTIAL_REPLAY_FORMAT)
            );
            let (event, state) = next(state).await.unwrap();
            let error = event.unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(error.delivery(), DeliveryState::Started);
            assert!(next(state).await.is_none());
        }
    }

    #[test]
    fn ignored_extensions_cannot_hide_invalid_utf8() {
        for frame in [
            b"{\"choices\":[{\"index\":0,\"delta\":{\"content\":\"text\"}}],\"extension\":\"\xff\"}".as_slice(),
            b"{\"choices\":false,\"error\":{\"code\":\"rate_limit_exceeded\"},\"extension\":\"\xff\"}".as_slice(),
        ] {
            let mut state = state(b"");
            let error = state.handle(frame).unwrap_err();
            assert_eq!(error.kind(), ModelErrorKind::InvalidProviderResponse);
            assert_eq!(error.delivery(), DeliveryState::Unknown);
            assert_eq!(*error.retry_hint(), RetryHint::Never);
            assert!(state.pending.is_empty());
            assert!(!state.seen);
        }
    }
}
