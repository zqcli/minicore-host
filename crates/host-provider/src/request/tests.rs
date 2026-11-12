use super::*;
use minicore_host_runtime::{
    ToolCallId,
    model::{ModelLimits, ProviderReplay, ReasoningContent, ToolCall},
    tools::{ToolOutput, ToolResultOutcome, ToolSpec},
};
use serde_json::json;
use std::collections::BTreeSet;

fn descriptor() -> ModelDescriptor {
    ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([ReasoningPreference::Auto, ReasoningPreference::High]),
        true,
    )
    .unwrap()
}

#[test]
fn chat_request_preserves_multipart_history_and_exact_optional_fields() {
    let id = ToolCallId::new("original-call").unwrap();
    let exact_numbers: Value = serde_json::from_str(
        r#"{"integer":18446744073709551616001,"decimal":0.12345678901234567890123456789}"#,
    )
    .unwrap();
    let arguments =
        json!({"quote": "\"\\\n世界", "nested": {"enabled": true}, "numbers": exact_numbers});
    let schema = json!({"type": "object", "properties": {"quote": {"type": "string"}}, "examples": [exact_numbers]});
    let request = ModelRequest::new(
        vec![
            ModelMessage::system("system\n世界").unwrap(),
            ModelMessage::user("first input").unwrap(),
            ModelMessage::assistant(vec![
                AssistantPart::Text("a\"".into()),
                AssistantPart::Reasoning(
                    ReasoningContent::new(Some("r\\one".into()), None, None, None).unwrap(),
                ),
                AssistantPart::Text("界".into()),
                AssistantPart::Reasoning(
                    ReasoningContent::new(Some("r\ntwo".into()), None, None, None).unwrap(),
                ),
                AssistantPart::ToolCall(
                    ToolCall::new(id.clone(), "echo".parse().unwrap(), arguments.clone(), 0)
                        .unwrap(),
                ),
            ])
            .unwrap(),
            ModelMessage::tool_with_outcome(
                id,
                ToolOutput::new("line\n二").unwrap(),
                ToolResultOutcome::Success,
            )
            .unwrap(),
            ModelMessage::assistant_with_provider_replay(
                vec![],
                Some(
                    ProviderReplay::new(
                        REPLAY_FORMAT,
                        json!({"finish_reason": "stop", "usage": {}}),
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
            ModelMessage::user("continue").unwrap(),
        ],
        vec![ToolSpec::new("echo".parse().unwrap(), "Echo \"quoted\"", schema.clone()).unwrap()],
        ModelLimits::new(Some(65_536), Some(123)).unwrap(),
        ReasoningPreference::High,
    )
    .unwrap();
    let actual: Value =
        serde_json::from_slice(&encode("fixture-model", &descriptor(), &request).unwrap()).unwrap();
    assert_eq!(
        actual,
        json!({
            "model": "fixture-model",
            "messages": [
                {"role": "system", "content": "system\n世界"},
                {"role": "user", "content": "first input"},
                {"role": "assistant", "content": "a\"界", "reasoning_content": "r\\oner\ntwo",
                 "tool_calls": [{"id": "original-call", "type": "function",
                    "function": {"name": "echo", "arguments": arguments.to_string()}}]},
                {"role": "tool", "tool_call_id": "original-call", "content": "line\n二"},
                {"role": "assistant", "content": null},
                {"role": "user", "content": "continue"}
            ],
            "tools": [{"type": "function", "function": {
                "name": "echo", "description": "Echo \"quoted\"", "parameters": schema
            }}],
            "stream": true,
            "stream_options": {"include_usage": true},
            "n": 1,
            "max_completion_tokens": 123,
            "reasoning_effort": "high"
        })
    );
}

#[test]
fn unsupported_replay_and_opaque_reasoning_remain_invalid_requests() {
    for (format, payload) in [
        ("foreign.provider", json!({"finish_reason": "stop"})),
        (
            REPLAY_FORMAT,
            json!({"finish_reason": "stop", "unknown": null}),
        ),
        (REPLAY_FORMAT, json!({"usage": {}})),
        (REPLAY_FORMAT, json!({"finish_reason": 1})),
        (REPLAY_FORMAT, json!({"finish_reason": "stop", "usage": []})),
    ] {
        let request = ModelRequest::new(
            vec![
                ModelMessage::user("input").unwrap(),
                ModelMessage::assistant_with_provider_replay(
                    vec![AssistantPart::Text("prior".into())],
                    Some(ProviderReplay::new(format, payload).unwrap()),
                )
                .unwrap(),
            ],
            vec![],
            ModelLimits::default(),
            ReasoningPreference::Auto,
        )
        .unwrap();
        assert_eq!(
            encode("fixture-model", &descriptor(), &request)
                .unwrap_err()
                .kind(),
            ModelErrorKind::InvalidRequest
        );
    }
    for reasoning in [
        ReasoningContent::new(None, Some("summary".into()), None, None).unwrap(),
        ReasoningContent::new(None, None, Some("encrypted".into()), None).unwrap(),
        ReasoningContent::new(Some("text".into()), None, None, Some("signature".into())).unwrap(),
    ] {
        let request = ModelRequest::new(
            vec![
                ModelMessage::user("input").unwrap(),
                ModelMessage::assistant(vec![AssistantPart::Reasoning(reasoning)]).unwrap(),
            ],
            vec![],
            ModelLimits::default(),
            ReasoningPreference::Auto,
        )
        .unwrap();
        assert_eq!(
            encode("fixture-model", &descriptor(), &request)
                .unwrap_err()
                .kind(),
            ModelErrorKind::InvalidRequest
        );
    }
}
