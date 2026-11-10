use minicore_runtime::model::{
    AssistantPart, ModelDescriptor, ModelError, ModelErrorKind, ModelMessage, ModelRequest,
    ReasoningPreference,
};
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;

use crate::{error, stream::REPLAY_FORMAT};

#[cfg(test)]
mod tests;

// These private projections borrow the immutable model request. Only the Chat
// format's concatenated assistant parts and JSON-encoded tool arguments need
// owned strings; no intermediate JSON tree owns another copy of the context.
#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    n: u8,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Definition<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum Message<'a> {
    System {
        content: &'a str,
    },
    User {
        content: &'a str,
    },
    Tool {
        tool_call_id: &'a str,
        content: &'a str,
    },
    Assistant {
        content: Option<Cow<'a, str>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<Cow<'a, str>>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<Call<'a>>,
    },
}

#[derive(Serialize)]
struct Definition<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionDefinition<'a>,
}

#[derive(Serialize)]
struct FunctionDefinition<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
}

#[derive(Serialize)]
struct Call<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionCall<'a>,
}

#[derive(Serialize)]
struct FunctionCall<'a> {
    name: &'a str,
    arguments: String,
}

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
    let tools = request
        .tools()
        .iter()
        .map(|spec| Definition {
            kind: "function",
            function: FunctionDefinition {
                name: spec.name().as_str(),
                description: spec.description().as_str(),
                parameters: spec.input_schema(),
            },
        })
        .collect();
    let reasoning_effort = match request.reasoning() {
        ReasoningPreference::Auto => None,
        ReasoningPreference::Disabled => Some("none"),
        ReasoningPreference::Low => Some("low"),
        ReasoningPreference::Medium => Some("medium"),
        ReasoningPreference::High => Some("high"),
        ReasoningPreference::XHigh => Some("xhigh"),
        ReasoningPreference::Max => Some("max"),
        ReasoningPreference::Ultra => Some("ultra"),
    };
    serde_json::to_vec(&Request {
        model,
        messages,
        stream: true,
        stream_options: StreamOptions {
            include_usage: true,
        },
        n: 1,
        tools,
        // No adapter-owned token quota. Only forward an explicit Runtime value.
        max_completion_tokens: request.limits().max_output_tokens(),
        reasoning_effort,
    })
    .map_err(|_| error::local(ModelErrorKind::InvalidRequest))
}

fn message(message: &ModelMessage) -> Result<Message<'_>, ModelError> {
    match message {
        ModelMessage::System(text) => Ok(Message::System { content: text }),
        ModelMessage::User(text) => Ok(Message::User { content: text }),
        ModelMessage::Tool {
            tool_call_id,
            output,
            ..
        } => Ok(Message::Tool {
            tool_call_id: tool_call_id.as_str(),
            content: output.content().as_str(),
        }),
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

fn assistant(parts: &[AssistantPart]) -> Result<Message<'_>, ModelError> {
    let mut text = Cow::Borrowed("");
    let mut reasoning = Cow::Borrowed("");
    let mut calls = Vec::new();
    for part in parts {
        match part {
            AssistantPart::Text(value) => append(&mut text, value),
            AssistantPart::Reasoning(value) => {
                if value.encrypted().is_some()
                    || value.signature().is_some()
                    || value.summary().is_some()
                {
                    // Chat's reasoning_content cannot represent these without
                    // changing their meaning. Require a compatible adapter.
                    return Err(error::local(ModelErrorKind::InvalidRequest));
                }
                if let Some(value) = value.text() {
                    append(&mut reasoning, value);
                }
            }
            AssistantPart::ToolCall(call) => calls.push(Call {
                id: call.tool_call_id().as_str(),
                kind: "function",
                function: FunctionCall {
                    name: call.name().as_str(),
                    arguments: serde_json::to_string(call.arguments())
                        .map_err(|_| error::local(ModelErrorKind::InvalidRequest))?,
                },
            }),
        }
    }
    Ok(Message::Assistant {
        content: (!text.is_empty()).then_some(text),
        reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
        tool_calls: calls,
    })
}

fn append<'a>(combined: &mut Cow<'a, str>, part: &'a str) {
    if combined.is_empty() {
        *combined = Cow::Borrowed(part);
    } else if !part.is_empty() {
        combined.to_mut().push_str(part);
    }
}
