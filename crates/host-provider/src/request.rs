use minicore_runtime::model::{
    AssistantPart, ModelDescriptor, ModelError, ModelErrorKind, ModelMessage, ModelRequest,
    ReasoningPreference,
};
use serde_json::{Value, json};

use crate::{error, stream::REPLAY_FORMAT};

pub(crate) fn encode(
    model: &str,
    descriptor: &ModelDescriptor,
    request: &ModelRequest,
) -> Result<Vec<u8>, ModelError> {
    if !descriptor.supports_reasoning(request.reasoning())
        || (!descriptor.supports_tools && !request.tools().is_empty())
    {
        return Err(error::local(ModelErrorKind::InvalidRequest));
    }
    let messages = request
        .messages()
        .iter()
        .map(message)
        .collect::<Result<Vec<_>, _>>()?;
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
        "n": 1,
    });
    if !request.tools().is_empty() {
        body["tools"] = Value::Array(
            request
                .tools()
                .iter()
                .map(|spec| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": spec.name().as_str(),
                            "description": spec.description().as_str(),
                            "parameters": spec.input_schema(),
                        }
                    })
                })
                .collect(),
        );
    }
    // No adapter-owned token quota. Only forward an explicit Runtime value.
    if let Some(tokens) = request.limits().max_output_tokens() {
        body["max_completion_tokens"] = json!(tokens);
    }
    let effort = match request.reasoning() {
        ReasoningPreference::Auto => None,
        ReasoningPreference::Disabled => Some("none"),
        ReasoningPreference::Low => Some("low"),
        ReasoningPreference::Medium => Some("medium"),
        ReasoningPreference::High => Some("high"),
        ReasoningPreference::XHigh => Some("xhigh"),
        ReasoningPreference::Max => Some("max"),
        ReasoningPreference::Ultra => Some("ultra"),
    };
    if let Some(effort) = effort {
        body["reasoning_effort"] = json!(effort);
    }
    serde_json::to_vec(&body).map_err(|_| error::local(ModelErrorKind::InvalidRequest))
}

fn message(message: &ModelMessage) -> Result<Value, ModelError> {
    match message {
        ModelMessage::System(text) => Ok(json!({"role": "system", "content": text})),
        ModelMessage::User(text) => Ok(json!({"role": "user", "content": text})),
        ModelMessage::Tool {
            tool_call_id,
            output,
            ..
        } => Ok(json!({
            "role": "tool", "tool_call_id": tool_call_id.as_str(), "content": output.content().as_str(),
        })),
        ModelMessage::Assistant(parts) => assistant(parts),
        ModelMessage::AssistantWithReplay {
            parts,
            provider_replay,
        } => {
            // We can replay our own finish metadata via canonical parts. Do not
            // silently drop another provider's encrypted/opaque history.
            if provider_replay.format() != REPLAY_FORMAT
                || provider_replay.payload().as_object().is_none_or(|object| {
                    object
                        .keys()
                        .any(|key| key != "finish_reason" && key != "usage")
                        || object
                            .get("finish_reason")
                            .is_none_or(|reason| !reason.is_string())
                        || object.get("usage").is_some_and(|usage| !usage.is_object())
                })
            {
                return Err(error::local(ModelErrorKind::InvalidRequest));
            }
            assistant(parts)
        }
    }
}

fn assistant(parts: &[AssistantPart]) -> Result<Value, ModelError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls = Vec::new();
    for part in parts {
        match part {
            AssistantPart::Text(value) => text.push_str(value),
            AssistantPart::Reasoning(value) => {
                if value.encrypted().is_some() || value.signature().is_some() || value.summary().is_some() {
                    // Chat's reasoning_content cannot represent these without
                    // changing their meaning. Require a compatible adapter.
                    return Err(error::local(ModelErrorKind::InvalidRequest));
                }
                if let Some(value) = value.text() { reasoning.push_str(value); }
            }
            AssistantPart::ToolCall(call) => calls.push(json!({
                "id": call.tool_call_id().as_str(), "type": "function",
                "function": { "name": call.name().as_str(), "arguments": serde_json::to_string(call.arguments())
                    .map_err(|_| error::local(ModelErrorKind::InvalidRequest))? },
            })),
        }
    }
    let mut result = json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) }});
    if !reasoning.is_empty() {
        result["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        result["tool_calls"] = Value::Array(calls);
    }
    Ok(result)
}
