//! Scripted loopback TCP fixtures exercising the production Responses adapter
//! and real Runtime driver. These tests do not call a live model service.

mod support;

use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use minicore_host_provider::{OpenAiResponsesConfig, OpenAiResponsesModel, SecretApiKey};
use minicore_host_runtime::{
    LoopId,
    model::{
        AssistantPart, DeliveryState, Model, ModelCallContext, ModelCallEvent, ModelCallParams,
        ModelCallReport, ModelDescriptor, ModelDriver, ModelDriverConfig, ModelErrorKind,
        ModelEvent, ModelFinishReason, ModelLimits, ModelMessage, ModelRequest, ModelResponse,
        ProviderReplay, ReasoningPreference, RetryHint,
    },
    tools::{ToolOutput, ToolResultOutcome, ToolSpec},
};
use serde_json::{Value, json};
use support::{Fixture, Reply};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn provider(endpoint: &str) -> Arc<OpenAiResponsesModel> {
    let descriptor = ModelDescriptor::new(
        "fixture/responses".parse().unwrap(),
        128_000,
        BTreeSet::from([
            ReasoningPreference::Auto,
            ReasoningPreference::High,
            ReasoningPreference::Max,
        ]),
        true,
    )
    .unwrap();
    Arc::new(
        OpenAiResponsesModel::new(
            OpenAiResponsesConfig::new(
                endpoint.replace("/chat/completions", "/responses"),
                "fixture-responses-model",
                descriptor,
            ),
            SecretApiKey::new("fixture-not-a-real-key").unwrap(),
        )
        .unwrap(),
    )
}

fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "echo".parse().unwrap(),
            "Echo declared input",
            json!({"type":"object","properties":{"value":{"type":"string"}}}),
        )
        .unwrap(),
    ]
}

fn request(tools: bool) -> ModelRequest {
    ModelRequest::new(
        vec![
            ModelMessage::system("Role instructions").unwrap(),
            ModelMessage::user("fixture input").unwrap(),
        ],
        if tools { specs() } else { vec![] },
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap()
}

async fn call(model: &dyn Model, request: ModelRequest) -> ModelCallReport {
    call_with(
        model,
        request,
        CancellationToken::new(),
        Duration::from_secs(5),
    )
    .await
}

async fn call_with(
    model: &dyn Model,
    request: ModelRequest,
    cancellation: CancellationToken,
    timeout: Duration,
) -> ModelCallReport {
    ModelDriver::new(
        model,
        ModelDriverConfig::new(timeout, 3, Duration::from_millis(1)),
    )
    .unwrap()
    .call(
        request,
        ModelCallParams {
            loop_id: LoopId::new().unwrap(),
            request_index: 0,
            cancellation,
            deadline: None,
        },
        &mut |_| {},
    )
    .await
}

fn answer(report: &ModelCallReport) -> &ModelResponse {
    report.outcome.as_ref().unwrap()
}

fn body(events: &[Value]) -> String {
    events
        .iter()
        .enumerate()
        .map(|(sequence, value)| {
            let mut value = value.clone();
            value["sequence_number"] = json!(sequence);
            format!(
                "event: {}\r\ndata: {value}\r\n\r\n",
                value["type"].as_str().unwrap()
            )
        })
        .collect()
}

#[derive(Default)]
struct Script {
    events: Vec<Value>,
    output: Vec<Value>,
}

impl Script {
    fn new() -> Self {
        Self {
            events: vec![
                json!({"type":"response.created","response":{"id":"resp_fixture","status":"in_progress","output":[]}}),
            ],
            output: vec![],
        }
    }
    fn message(&mut self, id: &str, phase: Option<&str>, deltas: &[&str]) {
        let index = self.output.len();
        let text = deltas.concat();
        let mut added = json!({"type":"message","id":id,"role":"assistant","status":"in_progress","content":[]});
        if let Some(phase) = phase {
            added["phase"] = json!(phase);
        }
        self.events
            .push(json!({"type":"response.output_item.added","output_index":index,"item":added}));
        self.events.push(json!({"type":"response.content_part.added","output_index":index,"item_id":id,"content_index":0,"part":{"type":"output_text","text":""}}));
        for delta in deltas {
            self.events.push(json!({"type":"response.output_text.delta","output_index":index,"item_id":id,"content_index":0,"delta":delta}));
        }
        self.events.push(json!({"type":"response.output_text.done","output_index":index,"item_id":id,"content_index":0,"text":text}));
        let part = json!({"type":"output_text","text":text});
        self.events.push(json!({"type":"response.content_part.done","output_index":index,"item_id":id,"content_index":0,"part":part}));
        let mut item = json!({"type":"message","id":id,"role":"assistant","status":"completed","content":[part]});
        if let Some(phase) = phase {
            item["phase"] = json!(phase);
        }
        self.events
            .push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
        self.output.push(item);
    }
    fn reasoning(&mut self, streamed: bool) {
        let index = self.output.len();
        self.events.push(json!({"type":"response.output_item.added","output_index":index,"item":{"type":"reasoning","id":"rs_original","summary":[],"encrypted_content":"provisional-cipher"}}));
        if streamed {
            self.events.push(json!({"type":"response.reasoning_summary_part.added","output_index":index,"item_id":"rs_original","summary_index":0,"part":{"type":"summary_text","text":""}}));
            for delta in ["Consider ", "🐈"] {
                self.events.push(json!({"type":"response.reasoning_summary_text.delta","output_index":index,"item_id":"rs_original","summary_index":0,"delta":delta}));
            }
            self.events.push(json!({"type":"response.reasoning_summary_text.done","output_index":index,"item_id":"rs_original","summary_index":0,"text":"Consider 🐈"}));
            self.events.push(json!({"type":"response.reasoning_summary_part.done","output_index":index,"item_id":"rs_original","summary_index":0,"part":{"type":"summary_text","text":"Consider 🐈"}}));
        }
        let item = json!({"type":"reasoning","id":"rs_original","summary":[{"type":"summary_text","text":"Consider 🐈"}],"encrypted_content":"final-cipher","status":"completed"});
        self.events
            .push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
        self.output.push(item);
    }
    fn add_call(&mut self, id: &str, call_id: &str) -> usize {
        let index = self.output.len();
        let item = json!({"type":"function_call","id":id,"call_id":call_id,"name":"echo","arguments":"","status":"in_progress"});
        self.events
            .push(json!({"type":"response.output_item.added","output_index":index,"item":item}));
        self.output.push(item);
        index
    }
    fn arguments(&mut self, index: usize, delta: &str) {
        self.events.push(json!({"type":"response.function_call_arguments.delta","output_index":index,"item_id":self.output[index]["id"],"delta":delta}));
        let arguments = self.output[index]["arguments"].as_str().unwrap().to_owned() + delta;
        self.output[index]["arguments"] = json!(arguments);
    }
    fn end_call(&mut self, index: usize) {
        self.events.push(json!({"type":"response.function_call_arguments.done","output_index":index,"item_id":self.output[index]["id"],"arguments":self.output[index]["arguments"]}));
        self.output[index]["status"] = json!("completed");
        self.events.push(json!({"type":"response.output_item.done","output_index":index,"item":self.output[index]}));
    }
    fn finish(&mut self, usage: Option<Value>) {
        let mut response = json!({"id":"resp_fixture","status":"completed","output":self.output});
        if let Some(usage) = usage {
            response["usage"] = usage;
        }
        self.events
            .push(json!({"type":"response.completed","response":response}));
    }
    fn body(&self) -> String {
        body(&self.events)
    }
}

fn final_text(text: &str) -> String {
    let mut script = Script::new();
    script.message("msg_final", Some("final_answer"), &[text]);
    script.finish(None);
    script.body()
}

fn replay_message(response: &ModelResponse) -> ModelMessage {
    ModelMessage::assistant_with_provider_replay(
        response.parts().to_vec(),
        response.provider_replay().cloned(),
    )
    .unwrap()
}

#[tokio::test]
async fn responses_streams_text_reasoning_usage_and_stateless_request() {
    let mut script = Script::new();
    script.reasoning(true);
    script.message("msg_original", Some("final_answer"), &["Hello ", "世界"]);
    let usage = json!({"input_tokens":15,"output_tokens":9,"total_tokens":24,"input_tokens_details":{"cached_tokens":2,"cache_write_tokens":1},"output_tokens_details":{"reasoning_tokens":3}});
    script.finish(Some(usage.clone()));
    let fixture = Fixture::start(vec![
        Reply::sse(script.body()),
        Reply::sse(final_text("again")),
    ])
    .await;
    let model = provider(&fixture.endpoint);
    let initial = ModelRequest::new(
        request(false).messages().to_vec(),
        vec![],
        ModelLimits::new(None, Some(256)).unwrap(),
        ReasoningPreference::Max,
    )
    .unwrap();
    let report = call(model.as_ref(), initial).await;
    let response = answer(&report);
    assert_eq!(response.finish_reason(), ModelFinishReason::Stop);
    assert!(
        matches!(&response.parts()[0], AssistantPart::Reasoning(value) if value.text() == Some("Consider 🐈"))
    );
    assert!(
        matches!(&response.parts()[1], AssistantPart::Text(value) if value.as_ref() == "Hello 世界")
    );
    let counts = response.usage().unwrap();
    assert_eq!(
        (
            counts.input_tokens(),
            counts.output_tokens(),
            counts.reasoning_tokens(),
            counts.cache_read_tokens(),
            counts.cache_write_tokens(),
            counts.provider_total_tokens()
        ),
        (Some(12), Some(6), Some(3), Some(2), Some(1), Some(24))
    );
    let replay = response.provider_replay().unwrap();
    assert_eq!(replay.payload()["usage"], usage);
    assert!(!replay.payload().to_string().contains("Hello"));
    assert!(!replay.payload().to_string().contains("provisional-cipher"));
    let next = ModelRequest::new(
        vec![
            ModelMessage::user("fixture input").unwrap(),
            replay_message(response),
            ModelMessage::user("continue").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    assert!(call(model.as_ref(), next).await.outcome.is_ok());
    let requests = fixture.finish().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[0].head.starts_with("POST /v1/responses HTTP/1.1"));
    let first = &requests[0].body;
    assert_eq!(first["stream"], true);
    assert_eq!(first["store"], false);
    assert_eq!(first["truncation"], "disabled");
    assert_eq!(first["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(first["reasoning"], json!({"effort":"max","summary":"auto"}));
    assert_eq!(first["max_output_tokens"], 256);
    assert!(first.get("previous_response_id").is_none());
    assert_eq!(first["input"][0]["role"], "system");
    assert!(!first.to_string().contains("fixture-not-a-real-key"));
    let input = &requests[1].body["input"];
    assert_eq!(input[1], script.output[0]);
    assert_eq!(input[2]["id"], "msg_original");
    assert_eq!(input[2]["phase"], "final_answer");
    assert_eq!(input[2]["content"][0]["text"], "Hello 世界");
    assert!(requests[1].body.get("max_output_tokens").is_none());
}

#[tokio::test]
async fn responses_parallel_calls_keep_original_ids_and_runtime_arguments() {
    let mut script = Script::new();
    script.reasoning(false);
    script.message("msg_comment", Some("commentary"), &["Checking"]);
    let a = script.add_call("fc_item_a", "call_original_a");
    let b = script.add_call("fc_item_b", "call_original_b");
    script.arguments(a, "{\"value\":");
    script.arguments(b, "{\"value\":\"second\"}");
    script.arguments(a, "\"🐈\"}");
    script.end_call(b);
    script.end_call(a);
    script.finish(None);
    let fixture = Fixture::start(vec![
        Reply::sse(script.body()),
        Reply::sse(final_text("done")),
    ])
    .await;
    let model = provider(&fixture.endpoint);
    let report = call(model.as_ref(), request(true)).await;
    let response = answer(&report);
    assert_eq!(response.finish_reason(), ModelFinishReason::ToolCalls);
    let calls: Vec<_> = response
        .parts()
        .iter()
        .filter_map(AssistantPart::as_tool_call)
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].tool_call_id().as_str(), "call_original_a");
    assert_eq!(calls[0].arguments(), &json!({"value":"🐈"}));
    assert_eq!(calls[1].call_index(), 1);
    let mut messages = vec![
        ModelMessage::user("fixture input").unwrap(),
        replay_message(response),
    ];
    for call in calls {
        messages.push(
            ModelMessage::tool_with_outcome(
                call.tool_call_id().clone(),
                ToolOutput::new("scripted tool result").unwrap(),
                ToolResultOutcome::Success,
            )
            .unwrap(),
        );
    }
    assert!(
        call(
            model.as_ref(),
            ModelRequest::new(
                messages,
                specs(),
                ModelLimits::default(),
                ReasoningPreference::Auto
            )
            .unwrap()
        )
        .await
        .outcome
        .is_ok()
    );
    let requests = fixture.finish().await;
    assert_eq!(requests[0].body["tools"][0]["type"], "function");
    assert_eq!(requests[0].body["tools"][0]["strict"], false);
    assert!(requests[0].body["tools"][0].get("function").is_none());
    let input = requests[1].body["input"].as_array().unwrap();
    assert_eq!(input[2]["phase"], "commentary");
    assert_eq!(input[3]["id"], "fc_item_a");
    assert_eq!(input[3]["call_id"], "call_original_a");
    assert_eq!(
        serde_json::from_str::<Value>(input[3]["arguments"].as_str().unwrap()).unwrap(),
        json!({"value":"🐈"})
    );
    assert_eq!(input[5]["type"], "function_call_output");
    assert_eq!(input[5]["call_id"], "call_original_a");
}

#[tokio::test]
async fn responses_replay_preserves_phase_across_normalized_multibyte_text() {
    let mut script = Script::new();
    script.message("msg_one", Some("commentary"), &["a\r"]);
    script.message("msg_two", Some("final_answer"), &["\nb\t\u{7}🐈"]);
    script.finish(None);
    let fixture = Fixture::start(vec![
        Reply::sse(script.body()),
        Reply::sse(final_text("done")),
    ])
    .await;
    let model = provider(&fixture.endpoint);
    let report = call(model.as_ref(), request(false)).await;
    let response = answer(&report);
    assert!(
        matches!(&response.parts()[0],AssistantPart::Text(text) if text.as_ref() == "a\nb\t 🐈")
    );
    let next = ModelRequest::new(
        vec![
            replay_message(response),
            ModelMessage::user("next").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    assert!(call(model.as_ref(), next).await.outcome.is_ok());
    let requests = fixture.finish().await;
    let input = &requests[1].body["input"];
    assert_eq!(input[0]["content"][0]["text"], "a\n");
    assert_eq!(input[1]["content"][0]["text"], "b\t 🐈");
    assert_eq!(input[0]["phase"], "commentary");
    assert_eq!(input[1]["phase"], "final_answer");
}

#[tokio::test]
async fn responses_runtime_rejects_arguments_once_after_native_terminal_metadata() {
    let mut script = Script::new();
    let a = script.add_call("fc_a", "call_a");
    let b = script.add_call("fc_b", "call_b");
    script.arguments(a, "{\"value\":\"valid\"}");
    let invalid = "{\"value\":\"raw\u{1}control\"}";
    script.arguments(b, invalid);
    script.end_call(a);
    script.end_call(b);
    script.finish(None);
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(true)).await;
    let failure = report.outcome.unwrap_err();
    assert_eq!(
        failure.error().kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    let partial = failure.partial.unwrap();
    assert!(
        matches!(&partial.content[0],AssistantPart::ToolCall(call) if call.tool_call_id().as_str() == "call_a")
    );
    assert_eq!(partial.incomplete_tool_calls[0].arguments_text, invalid);
    assert_eq!(
        partial.provider_replay.unwrap().format(),
        "minicore-host.openai-responses.v1"
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn responses_incomplete_failed_and_eof_keep_partial_without_tool_end() {
    for terminal in [
        "response.incomplete",
        "response.failed",
        "eof",
        "done_sentinel",
    ] {
        let mut script = Script::new();
        script.reasoning(true);
        script.message("msg_partial", Some("commentary"), &["actual partial"]);
        let index = script.add_call("fc_partial", "call_partial");
        script.arguments(index, "{\"value\":");
        if terminal != "eof" && terminal != "done_sentinel" {
            let mut response = json!({"id":"resp_fixture","status":if terminal == "response.failed" {"failed"} else {"incomplete"},"output":[],"usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}});
            if terminal == "response.failed" {
                response["error"] = json!({"code":"server_error","message":"private failure text"});
            } else {
                response["incomplete_details"] = json!({"reason":"max_output_tokens"});
            }
            script
                .events
                .push(json!({"type":terminal,"response":response}));
        }
        let mut text = script.body();
        if terminal == "done_sentinel" {
            text.push_str("data: [DONE]\n\n");
        }
        let fixture = Fixture::start(vec![Reply::sse(text)]).await;
        let report = call(provider(&fixture.endpoint).as_ref(), request(true)).await;
        assert_eq!(report.attempts, 1);
        let failure = report.outcome.unwrap_err();
        assert_eq!(failure.error().delivery(), DeliveryState::Started);
        let partial = failure.partial.unwrap();
        assert!(partial.content.iter().any(
            |part| matches!(part,AssistantPart::Text(text) if text.as_ref() == "actual partial")
        ));
        assert!(
            !partial
                .content
                .iter()
                .any(|part| matches!(part, AssistantPart::ToolCall(_)))
        );
        assert_eq!(
            partial.incomplete_tool_calls[0].arguments_text,
            "{\"value\":"
        );
        let replay = partial.provider_replay.unwrap();
        assert_eq!(replay.format(), "minicore-host.openai-responses.partial.v1");
        assert!(replay.payload().to_string().contains("final-cipher"));
        assert!(
            !replay
                .payload()
                .to_string()
                .contains("private failure text")
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn responses_done_snapshots_never_append_missing_or_changed_output() {
    for target in [
        "response.output_text.done",
        "response.content_part.done",
        "response.output_item.done",
        "response.completed",
    ] {
        let mut script = Script::new();
        script.message("msg_snapshot", None, &["received"]);
        script.finish(None);
        let event = script
            .events
            .iter_mut()
            .find(|event| event["type"] == target)
            .unwrap();
        match target {
            "response.output_text.done" => event["text"] = json!("received fabricated suffix"),
            "response.content_part.done" => event["part"]["text"] = json!("changed"),
            "response.output_item.done" => event["item"]["content"][0]["text"] = json!("changed"),
            _ => event["response"]["output"][0]["content"][0]["text"] = json!("changed"),
        }
        let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
        let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
        let failure = report.outcome.unwrap_err();
        assert_eq!(
            failure.error().kind(),
            ModelErrorKind::InvalidProviderResponse
        );
        assert!(
            matches!(&failure.partial.unwrap().content[0],AssistantPart::Text(text) if text.as_ref() == "received")
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn responses_mismatched_argument_snapshot_keeps_original_fragments() {
    let mut script = Script::new();
    let index = script.add_call("fc_snapshot", "call_snapshot");
    script.arguments(index, "{\"value\":\"original\"}");
    script.end_call(index);
    script
        .events
        .iter_mut()
        .find(|event| event["type"] == "response.function_call_arguments.done")
        .unwrap()["arguments"] = json!("{\"value\":\"changed\"}");
    script.finish(None);
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(true)).await;
    let failure = report.outcome.unwrap_err();
    assert_eq!(
        failure.error().kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert_eq!(
        failure.partial.unwrap().incomplete_tool_calls[0].arguments_text,
        "{\"value\":\"original\"}"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn responses_rejects_duplicate_fields_and_mismatched_event_identity() {
    let mut script = Script::new();
    script.message("msg_identity", None, &["received"]);
    script.finish(None);
    let good = script.body();
    let cases = [
        good.replace(
            "\"delta\":\"received\"",
            "\"delta\":\"received\",\"delta\":\"other\"",
        ),
        good.replacen("\"item_id\":\"msg_identity\"", "\"item_id\":\"wrong\"", 1),
        good.replacen("\"content_index\":0", "\"content_index\":1", 1),
        good.replacen("\"output_index\":0", "\"output_index\":2", 1),
        good.replace("\"sequence_number\":3", "\"sequence_number\":2"),
        good.replacen("\"type\":\"output_text\"", "\"type\":\"output_image\"", 1),
        good.replacen("\"status\":\"completed\"", "\"status\":\"in_progress\"", 1),
    ];
    for text in cases {
        assert_ne!(text, good);
        let fixture = Fixture::start(vec![Reply::sse(text)]).await;
        let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
        assert_eq!(
            report.outcome.unwrap_err().error().kind(),
            ModelErrorKind::InvalidProviderResponse
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn responses_initial_errors_share_classification_and_retry_lower_bound() {
    let rejection = Reply::Full { status:200,content_type:"text/event-stream",headers:"Retry-After-ms: 40\r\n".into(),body:"data: {\"type\":\"error\",\"code\":\"rate_limit_exceeded\",\"message\":\"private\"}\n\n".into(),fragment:0 };
    let fixture = Fixture::start(vec![rejection, Reply::sse(final_text("recovered"))]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
    assert!(report.outcome.is_ok());
    assert_eq!(report.attempts, 2);
    assert!(report.retry_wait_elapsed >= Duration::from_millis(35));
    assert_eq!(fixture.finish().await.len(), 2);
    for (payload, kind, delivery) in [
        (
            json!({"error":{"type":"rate_limit_error","code":"insufficient_quota"},"response":false}),
            ModelErrorKind::QuotaExceeded,
            DeliveryState::NotStarted,
        ),
        (
            json!({"type":"error","code":"unrecognized"}),
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::Unknown,
        ),
        (
            json!({"type":"error","code":"invalid_api_key"}),
            ModelErrorKind::AuthRejected,
            DeliveryState::NotStarted,
        ),
    ] {
        let fixture = Fixture::start(vec![Reply::sse(format!("data: {payload}\n\n"))]).await;
        let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
        assert_eq!(report.attempts, 1);
        let failure = report.outcome.unwrap_err();
        assert_eq!(failure.error().kind(), kind);
        assert_eq!(failure.error().delivery(), delivery);
        assert_eq!(*failure.error().retry_hint(), RetryHint::Never);
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn responses_created_then_rejection_never_retries() {
    let mut script = Script::new();
    script
        .events
        .push(json!({"type":"error","code":"rate_limit_exceeded"}));
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
    assert_eq!(report.attempts, 1);
    assert_eq!(
        report.outcome.unwrap_err().error().delivery(),
        DeliveryState::Started
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn responses_failure_envelope_output_evidence_forbids_retry() {
    let reply = json!({"type":"response.failed","response":{"id":"resp_failed","status":"failed","error":{"code":"rate_limit_exceeded"},"output":[{"type":"unsupported-output"}]}});
    let fixture = Fixture::start(vec![Reply::sse(format!("data: {reply}\n\n"))]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
    assert_eq!(report.attempts, 1);
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::RateLimited);
    assert_eq!(failure.error().delivery(), DeliveryState::Started);
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn responses_cancel_while_waiting_for_http_headers_is_unknown_delivery() {
    let received = Arc::new(Notify::new());
    let fixture = Fixture::start(vec![Reply::HeadersHold {
        received: received.clone(),
    }])
    .await;
    let model = provider(&fixture.endpoint);
    let cancellation = CancellationToken::new();
    let token = cancellation.clone();
    let task = tokio::spawn(async move {
        call_with(
            model.as_ref(),
            request(false),
            token,
            Duration::from_secs(5),
        )
        .await
    });
    received.notified().await;
    cancellation.cancel();
    let report = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.attempts, 1);
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Cancelled);
    assert_eq!(failure.error().delivery(), DeliveryState::Unknown);
    assert!(failure.partial.is_none());
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn responses_cancellation_and_deadline_drop_real_stream_and_keep_partial() {
    for cancel in [true, false] {
        let mut script = Script::new();
        script.message("msg_wait", None, &["received before wait"]);
        let written = Arc::new(Notify::new());
        let disconnected = Arc::new(Notify::new());
        let fixture = Fixture::start(vec![Reply::Hold {
            prefix: script.body(),
            written: written.clone(),
            disconnected: disconnected.clone(),
        }])
        .await;
        let model = provider(&fixture.endpoint);
        let cancellation = CancellationToken::new();
        let token = cancellation.clone();
        let observed = Arc::new(Notify::new());
        let observed_by_driver = observed.clone();
        let task = tokio::spawn(async move {
            let timeout = if cancel {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(1)
            };
            ModelDriver::new(
                model.as_ref(),
                ModelDriverConfig::new(timeout, 3, Duration::from_millis(1)),
            )
            .unwrap()
            .call(
                request(false),
                ModelCallParams {
                    loop_id: LoopId::new().unwrap(),
                    request_index: 0,
                    cancellation: token,
                    deadline: None,
                },
                &mut |event| {
                    if matches!(event, ModelCallEvent::TextDelta(_)) {
                        observed_by_driver.notify_one();
                    }
                },
            )
            .await
        });
        written.notified().await;
        tokio::time::timeout(Duration::from_secs(2), observed.notified())
            .await
            .unwrap();
        if cancel {
            cancellation.cancel();
        }
        let report = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(report.attempts, 1);
        let failure = report.outcome.unwrap_err();
        assert_eq!(
            failure.error().kind(),
            if cancel {
                ModelErrorKind::Cancelled
            } else {
                ModelErrorKind::Timeout
            }
        );
        assert!(
            matches!(&failure.partial.unwrap().content[0],AssistantPart::Text(text) if text.as_ref() == "received before wait")
        );
        tokio::time::timeout(Duration::from_secs(2), disconnected.notified())
            .await
            .unwrap();
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn responses_cancelled_start_and_incompatible_replay_send_nothing() {
    let fixture = Fixture::start(vec![]).await;
    let model = provider(&fixture.endpoint);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = model
        .start(
            request(false),
            ModelCallContext::new(
                LoopId::new().unwrap(),
                0,
                cancellation,
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind(), ModelErrorKind::Cancelled);
    for format in [
        "minicore-host.openai-chat.v1",
        "minicore-host.openai-responses.partial.v1",
    ] {
        let replay = ProviderReplay::new(format, json!({"finish_reason":"stop"})).unwrap();
        let message = ModelMessage::assistant_with_provider_replay(
            vec![AssistantPart::Text("saved".into())],
            Some(replay),
        )
        .unwrap();
        let request = ModelRequest::new(
            vec![message, ModelMessage::user("continue").unwrap()],
            vec![],
            ModelLimits::default(),
            ReasoningPreference::Auto,
        )
        .unwrap();
        assert_eq!(
            call(model.as_ref(), request)
                .await
                .outcome
                .unwrap_err()
                .error()
                .kind(),
            ModelErrorKind::InvalidRequest
        );
    }
    assert_eq!(fixture.request_count(), 0);
    assert!(fixture.finish().await.is_empty());
}

#[tokio::test]
async fn responses_native_metadata_is_delivered_before_any_tool_end() {
    let mut script = Script::new();
    let index = script.add_call("fc_order", "call_order");
    script.arguments(index, "{}");
    script.end_call(index);
    script.finish(None);
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let model = provider(&fixture.endpoint);
    let mut stream = model
        .start(
            request(true),
            ModelCallContext::new(
                LoopId::new().unwrap(),
                0,
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
            ),
        )
        .await
        .unwrap();
    let mut replay = false;
    let mut ended = false;
    let mut finished = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ModelEvent::ProviderReplay { .. } => replay = true,
            ModelEvent::ToolCallEnd { .. } => {
                assert!(replay);
                ended = true;
            }
            ModelEvent::Finish { .. } => {
                assert!(ended);
                finished = true;
            }
            _ => {}
        }
    }
    assert!(finished);
    fixture.finish().await;
}

#[tokio::test]
async fn responses_invalid_usage_preserves_raw_counts_and_actual_text() {
    let mut script = Script::new();
    script.message("msg_usage", None, &["actual"]);
    script.finish(Some(
        json!({"input_tokens":1,"output_tokens":1,"total_tokens":200}),
    ));
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
    let failure = report.outcome.unwrap_err();
    assert_eq!(
        failure.error().kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    let partial = failure.partial.unwrap();
    assert!(partial.usage.is_none());
    assert_eq!(
        partial.provider_replay.unwrap().payload()["usage"]["total_tokens"],
        200
    );
    fixture.finish().await;
}

#[tokio::test]
async fn responses_invalid_tool_header_remains_non_executable_partial_metadata() {
    let mut script = Script::new();
    script.events.push(json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_bad","call_id":"call_bad","name":"invalid.name","arguments":"{\"raw\":","status":"in_progress"}}));
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(true)).await;
    let partial = report.outcome.unwrap_err().partial.unwrap();
    assert!(partial.content.is_empty());
    assert!(partial.incomplete_tool_calls.is_empty());
    assert_eq!(
        partial.provider_replay.unwrap().payload()["incomplete_tool_header"]["arguments"],
        "{\"raw\":"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn responses_encrypted_only_reasoning_round_trips_without_fake_text() {
    let mut script = Script::new();
    script.events.push(json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_only","summary":[],"encrypted_content":"unfinished"}}));
    let item = json!({"type":"reasoning","id":"rs_only","summary":[],"encrypted_content":"actual-complete-cipher"});
    script
        .events
        .push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
    script.output.push(item.clone());
    script.finish(None);
    let fixture = Fixture::start(vec![
        Reply::sse(script.body()),
        Reply::sse(final_text("next")),
    ])
    .await;
    let model = provider(&fixture.endpoint);
    let report = call(model.as_ref(), request(false)).await;
    let response = answer(&report);
    assert!(response.parts().is_empty());
    let request = ModelRequest::new(
        vec![
            replay_message(response),
            ModelMessage::user("next").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    assert!(call(model.as_ref(), request).await.outcome.is_ok());
    let requests = fixture.finish().await;
    assert_eq!(requests[1].body["input"][0], item);
}

#[tokio::test]
async fn responses_empty_completion_and_wrong_terminal_status_never_succeed() {
    for reply in [
        json!({"id":"resp_fixture","status":"completed","output":[]}),
        json!({"id":"resp_fixture","status":"incomplete","output":[]}),
    ] {
        let mut script = Script::new();
        script
            .events
            .push(json!({"type":"response.completed","response":reply}));
        let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
        let report = call(provider(&fixture.endpoint).as_ref(), request(false)).await;
        assert_eq!(
            report.outcome.unwrap_err().error().kind(),
            ModelErrorKind::InvalidProviderResponse
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn responses_replay_rejects_mismatched_model_canonical_text_and_ranges() {
    let fixture = Fixture::start(vec![Reply::sse(final_text("saved"))]).await;
    let model = provider(&fixture.endpoint);
    let report = call(model.as_ref(), request(false)).await;
    let response = answer(&report);
    for problem in ["model", "status", "text", "ranges"] {
        let mut payload = response.provider_replay().unwrap().payload().clone();
        let mut parts = response.parts().to_vec();
        match problem {
            "model" => payload["model"] = json!("different-model"),
            "status" => payload["status"] = json!("incomplete"),
            "text" => parts = vec![AssistantPart::Text("other".into())],
            _ => {
                payload["items"][0]["content"][0]["ranges"] =
                    json!([{"start":0,"end":5},{"start":0,"end":5}])
            }
        }
        let replay =
            ProviderReplay::new(response.provider_replay().unwrap().format(), payload).unwrap();
        let message = ModelMessage::assistant_with_provider_replay(parts, Some(replay)).unwrap();
        let request = ModelRequest::new(
            vec![message, ModelMessage::user("next").unwrap()],
            vec![],
            ModelLimits::default(),
            ReasoningPreference::Auto,
        )
        .unwrap();
        assert_eq!(
            call(model.as_ref(), request)
                .await
                .outcome
                .unwrap_err()
                .error()
                .kind(),
            ModelErrorKind::InvalidRequest
        );
    }
    assert_eq!(fixture.finish().await.len(), 1);
}
