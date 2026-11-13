use std::{borrow::Cow, collections::BTreeSet};

use minicore_host_runtime::model::{
    AssistantPart, ModelDescriptor, ModelError, ModelErrorKind, ModelMessage, ModelRequest,
    ReasoningPreference, ToolCall,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    replay::{self, Fingerprint, Replay, ReplayItem, TextRange},
    wire,
};
use crate::error;

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: Vec<Input<'a>>,
    stream: bool,
    store: bool,
    truncation: &'static str,
    include: [&'static str; 1],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Definition<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning>,
}

#[derive(Serialize)]
struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Input<'a> {
    Message(Message<'a>),
    Call(Call<'a>),
    Output(Output<'a>),
    Reasoning(wire::Item),
}

#[derive(Serialize)]
struct Message<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<String>,
    content: Vec<Content<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content<'a> {
    InputText {
        text: &'a str,
    },
    OutputText {
        text: Cow<'a, str>,
        annotations: [(); 0],
    },
    Refusal {
        refusal: Cow<'a, str>,
    },
}

#[derive(Serialize)]
struct Call<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    call_id: &'a str,
    name: &'a str,
    arguments: String,
}

#[derive(Serialize)]
struct Output<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: &'a str,
    output: &'a str,
}

#[derive(Serialize)]
struct Definition<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
    // Do not silently strengthen the Host's already-declared JSON schema.
    strict: bool,
}

fn invalid() -> ModelError {
    error::local(ModelErrorKind::InvalidRequest)
}

pub(super) fn encode(
    model: &str,
    descriptor: &ModelDescriptor,
    request: &ModelRequest,
) -> Result<Vec<u8>, ModelError> {
    if !descriptor.supports_reasoning(request.reasoning())
        || (!descriptor.supports_tools && !request.tools().is_empty())
    {
        return Err(invalid());
    }
    let mut input = Vec::new();
    for message in request.messages() {
        match message {
            ModelMessage::System(text) | ModelMessage::User(text) => {
                input.push(Input::Message(Message {
                    kind: "message",
                    role: if matches!(message, ModelMessage::System(_)) {
                        "system"
                    } else {
                        "user"
                    },
                    status: None,
                    id: None,
                    phase: None,
                    content: vec![Content::InputText { text }],
                }))
            }
            ModelMessage::Tool {
                tool_call_id,
                output,
                ..
            } => input.push(Input::Output(Output {
                kind: "function_call_output",
                call_id: tool_call_id.as_str(),
                output: output.content().as_str(),
            })),
            ModelMessage::Assistant(parts) => {
                for part in parts {
                    match part {
                        AssistantPart::Text(text) => input.push(Input::Message(Message {
                            kind: "message",
                            role: "assistant",
                            status: Some("completed"),
                            id: None,
                            phase: None,
                            content: vec![Content::OutputText {
                                text: Cow::Borrowed(text),
                                annotations: [],
                            }],
                        })),
                        AssistantPart::ToolCall(call) => {
                            input.push(Input::Call(call_input(call, None)?))
                        }
                        // Native reasoning requires its actual item identity. Never
                        // discard it or manufacture a reasoning ID from display text.
                        AssistantPart::Reasoning(_) => return Err(invalid()),
                    }
                }
            }
            ModelMessage::AssistantWithReplay {
                parts,
                provider_replay,
            } => {
                if provider_replay.format() != replay::FORMAT {
                    return Err(invalid());
                }
                let replay =
                    Replay::deserialize(provider_replay.payload()).map_err(|_| invalid())?;
                input.extend(project(model, parts, replay)?);
            }
        }
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
    let summary = (request.reasoning() != ReasoningPreference::Disabled
        && descriptor
            .supported_reasoning
            .iter()
            .any(|p| !matches!(p, ReasoningPreference::Auto | ReasoningPreference::Disabled)))
    .then_some("auto");
    let body = Request {
        model,
        input,
        stream: true,
        store: false,
        truncation: "disabled",
        include: ["reasoning.encrypted_content"],
        max_output_tokens: request.limits().max_output_tokens(),
        reasoning: (effort.is_some() || summary.is_some()).then_some(Reasoning { effort, summary }),
        tools: request
            .tools()
            .iter()
            .map(|tool| Definition {
                kind: "function",
                name: tool.name().as_str(),
                description: tool.description().as_str(),
                parameters: tool.input_schema(),
                strict: false,
            })
            .collect(),
    };
    serde_json::to_vec(&body).map_err(|_| invalid())
}

fn call_input(call: &ToolCall, id: Option<String>) -> Result<Call<'_>, ModelError> {
    Ok(Call {
        kind: "function_call",
        status: "completed",
        id,
        call_id: call.tool_call_id().as_str(),
        name: call.name().as_str(),
        arguments: serde_json::to_string(call.arguments()).map_err(|_| invalid())?,
    })
}

struct CanonicalText<'a> {
    parts: Vec<&'a str>,
    fingerprint: Fingerprint,
}

impl<'a> CanonicalText<'a> {
    fn new() -> Self {
        Self {
            parts: Vec::new(),
            fingerprint: Fingerprint::default(),
        }
    }
    fn push(&mut self, text: &'a str) -> Result<(), ModelError> {
        self.fingerprint.push(text).map_err(|_| invalid())?;
        self.parts.push(text);
        Ok(())
    }
    fn slice(&self, range: TextRange) -> Result<Cow<'a, str>, ModelError> {
        if range.start >= range.end || range.end > self.fingerprint.bytes {
            return Err(invalid());
        }
        let mut offset: usize = 0;
        let mut result = Cow::Borrowed("");
        for text in &self.parts {
            let end = offset.checked_add(text.len()).ok_or_else(invalid)?;
            if range.start < end && range.end > offset {
                let start = range.start.saturating_sub(offset);
                let finish = range.end.min(end) - offset;
                let value = text.get(start..finish).ok_or_else(invalid)?;
                if result.is_empty() {
                    result = Cow::Borrowed(value);
                } else {
                    result.to_mut().push_str(value);
                }
            }
            offset = end;
        }
        Ok(result)
    }
}

fn project<'a>(
    model: &str,
    parts: &'a [AssistantPart],
    replay: Replay,
) -> Result<Vec<Input<'a>>, ModelError> {
    if replay.model != model
        || replay.status.as_deref() != Some("completed")
        || replay
            .response_id
            .as_deref()
            .is_none_or(|id| !wire::valid_id(id))
        || replay.incomplete_details.is_some()
        || replay.incomplete_tool_header.is_some()
        || replay.items.is_empty()
    {
        return Err(invalid());
    }
    let mut text = CanonicalText::new();
    let mut reasoning = CanonicalText::new();
    let mut calls = Vec::new();
    for part in parts {
        match part {
            AssistantPart::Text(value) => text.push(value)?,
            AssistantPart::Reasoning(value) => {
                if value.summary().is_some()
                    || value.encrypted().is_some()
                    || value.signature().is_some()
                {
                    return Err(invalid());
                }
                if let Some(value) = value.text() {
                    reasoning.push(value)?;
                }
            }
            AssistantPart::ToolCall(call) => calls.push(call),
        }
    }
    if text.fingerprint.checksum() != replay.text
        || reasoning.fingerprint.checksum() != replay.reasoning
    {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    let mut used_ranges = Vec::new();
    let mut call_index = 0;
    let mut input = Vec::new();
    for item in replay.items {
        match item {
            ReplayItem::Message { id, phase, content } => {
                if !wire::valid_id(&id)
                    || !ids.insert(id.clone())
                    || !wire::valid_phase(phase.as_deref())
                    || content.is_empty()
                {
                    return Err(invalid());
                }
                let mut projected = Vec::new();
                for part in content {
                    let mut value = Cow::Borrowed("");
                    for range in part.ranges {
                        let slice = text.slice(range)?;
                        used_ranges.push(range);
                        if value.is_empty() {
                            value = slice;
                        } else {
                            value.to_mut().push_str(&slice);
                        }
                    }
                    projected.push(match part.kind.as_str() {
                        "output_text" => Content::OutputText {
                            text: value,
                            annotations: [],
                        },
                        "refusal" => Content::Refusal { refusal: value },
                        _ => return Err(invalid()),
                    });
                }
                input.push(Input::Message(Message {
                    kind: "message",
                    role: "assistant",
                    status: Some("completed"),
                    id: Some(id),
                    phase,
                    content: projected,
                }));
            }
            ReplayItem::FunctionCall { id, call_id, name } => {
                if !wire::valid_id(&id) || !ids.insert(id.clone()) {
                    return Err(invalid());
                }
                let call = calls.get(call_index).ok_or_else(invalid)?;
                if call.tool_call_id().as_str() != call_id || call.name().as_str() != name {
                    return Err(invalid());
                }
                input.push(Input::Call(call_input(call, Some(id))?));
                call_index += 1;
            }
            ReplayItem::Reasoning { item } => {
                wire::validate_reasoning(&item).map_err(|_| invalid())?;
                if !ids.insert(item.id.clone()) {
                    return Err(invalid());
                }
                input.push(Input::Reasoning(item));
            }
        }
    }
    used_ranges.sort_by_key(|range| range.start);
    let mut end = 0;
    for range in used_ranges {
        if range.start != end {
            return Err(invalid());
        }
        end = range.end;
    }
    if end != text.fingerprint.bytes || call_index != calls.len() {
        return Err(invalid());
    }
    Ok(input)
}
