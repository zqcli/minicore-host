mod support;

use futures_util::StreamExt;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_runtime::{
    AgentLoop, CancelReason, ExecutionConfig, HistoryItem, LoopEvent, LoopId, LoopInputs,
    LoopOptions, LoopOutcome, LoopReport, LoopRequest, UserInput,
    model::{
        AssistantPart, DeliveryState, Model, ModelCallContext, ModelDescriptor, ModelError,
        ModelErrorKind, ModelEvent, ModelFinishReason, ModelLimits, ModelMessage, ModelRequest,
        ModelStartFuture, ReasoningPreference, RetryHint,
    },
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet,
        ToolSpec,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use support::{Fixture, Reply};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

fn frame(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"id": "fixture-response", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    )
}
fn final_text(text: &str) -> String {
    frame(json!({"content": text}), None) + &frame(json!({}), Some("stop")) + "data: [DONE]\n\n"
}
fn provider(endpoint: &str) -> Arc<OpenAiChatModel> {
    let descriptor = ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([ReasoningPreference::Auto, ReasoningPreference::High]),
        true,
    )
    .unwrap();
    Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(endpoint, "fixture-only-model", descriptor),
            SecretApiKey::new("fixture-not-a-real-key").unwrap(),
        )
        .unwrap(),
    )
}
struct NoInputs;
impl LoopInputs for NoInputs {
    fn take(&self) -> Vec<UserInput> {
        Vec::new()
    }
}
fn start(model: Arc<dyn Model>, tools: ToolSet) -> AgentLoop {
    let mut options = LoopOptions::default_checked().unwrap();
    options.model_retry_base_delay = Duration::from_millis(1);
    options.model_timeout = Duration::from_secs(3);
    start_with_options(model, tools, options)
}
fn start_with_options(model: Arc<dyn Model>, tools: ToolSet, options: LoopOptions) -> AgentLoop {
    let config = ExecutionConfig::new(
        model,
        ReasoningPreference::Auto,
        tools,
        None,
        Arc::new(DefaultPromptProvider::new(Some(
            minicore_runtime::BoundedText::new("Framework and role").unwrap(),
        ))),
    )
    .unwrap();
    AgentLoop::start(
        LoopRequest::new(
            Arc::from([]),
            UserInput::text("fixture user request").unwrap(),
            config,
        ),
        options,
        Arc::new(NoInputs),
    )
    .unwrap()
}
async fn finish(agent: AgentLoop) -> Arc<LoopReport> {
    tokio::time::timeout(Duration::from_secs(5), agent.join())
        .await
        .unwrap()
        .unwrap()
}
fn assistant(report: &LoopReport, index: usize) -> &minicore_runtime::AssistantHistory {
    report
        .appended
        .iter()
        .filter_map(|item| match item {
            HistoryItem::Assistant(value) => Some(value),
            _ => None,
        })
        .nth(index)
        .unwrap()
}
fn failure(report: &LoopReport) -> &minicore_runtime::model::ModelError {
    let LoopOutcome::Failed(failure) = &report.outcome else {
        panic!("not failed: {:?}", report.outcome)
    };
    failure.model_error().unwrap()
}
fn context(cancellation: CancellationToken) -> ModelCallContext {
    ModelCallContext::new(
        LoopId::new().unwrap(),
        0,
        cancellation,
        Instant::now() + Duration::from_secs(3),
    )
}
fn request() -> ModelRequest {
    ModelRequest::new(
        vec![ModelMessage::user("fixture input").unwrap()],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap()
}

#[tokio::test]
async fn real_http_sse_streams_text_reasoning_usage_and_exact_request() {
    let usage = json!({"prompt_tokens": 12, "completion_tokens": 9, "total_tokens": 21,
        "prompt_tokens_details": {"cached_tokens": 2}, "completion_tokens_details": {"reasoning_tokens": 3}});
    let body = ": keepalive\r\n\r\n".to_owned()
        + &frame(
            json!({"role": "assistant", "reasoning_content": "think 🐈"}),
            None,
        )
        + &frame(json!({"content": "hello 世界"}), None)
        + &frame(json!({}), Some("stop"))
        + &format!("data: {}\r\n\r\n", json!({"choices": [], "usage": usage}))
        + "data: [DONE]\r\n\r\n";
    let fixture = Fixture::start(vec![Reply::sse(body)]).await;
    let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    let answer = assistant(&report, 0);
    assert_eq!(answer.finish_reason, ModelFinishReason::Stop);
    assert!(
        matches!(&answer.content[0], AssistantPart::Reasoning(value) if value.text() == Some("think 🐈"))
    );
    assert!(
        matches!(&answer.content[1], AssistantPart::Text(value) if value.as_ref() == "hello 世界")
    );
    assert_eq!(answer.usage.input_tokens(), Some(10));
    assert_eq!(answer.usage.output_tokens(), Some(6));
    assert_eq!(answer.usage.reasoning_tokens(), Some(3));
    assert_eq!(answer.usage.cache_read_tokens(), Some(2));
    assert_eq!(answer.usage.provider_total_tokens(), Some(21));
    assert_eq!(
        answer.provider_replay.as_ref().unwrap().payload()["usage"],
        usage
    );
    assert!(
        !serde_json::to_string(&report.appended)
            .unwrap()
            .contains("fixture-not-a-real-key")
    );
    let requests = fixture.finish().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert!(
        request
            .head
            .starts_with("POST /v1/chat/completions HTTP/1.1")
    );
    assert!(
        request
            .head
            .to_lowercase()
            .contains("authorization: bearer fixture-not-a-real-key")
    );
    assert_eq!(request.body["model"], "fixture-only-model");
    assert_eq!(
        request.body["messages"][0],
        json!({"role":"system", "content":"Framework and role"})
    );
    assert_eq!(
        request.body["messages"][1]["content"],
        "fixture user request"
    );
    assert_eq!(request.body["stream_options"]["include_usage"], true);
    assert!(request.body.get("max_completion_tokens").is_none());
    assert!(!request.body.to_string().contains("fixture-not-a-real-key"));
}

struct Echo {
    spec: ToolSpec,
    calls: Arc<Mutex<Vec<ToolInvocation>>>,
}
impl Tool for Echo {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.calls.lock().unwrap().push(invocation.clone());
        Box::pin(async move {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new(format!("echo:{}", invocation.arguments["value"])).unwrap(),
            ))
        })
    }
}

#[tokio::test]
async fn runtime_executes_parallel_tools_once_and_sends_original_ids_and_history() {
    let id1 = "call_provider_original_".to_owned() + &"x".repeat(90);
    let id2 = "call_OTHER";
    let first = frame(json!({"reasoning_content": "use tools"}), None)
        + &frame(
            json!({"tool_calls":[{"index":0,"id":&id1[..10],"type":"function","function":{"name":"ec"}}]}),
            None,
        )
        + &frame(
            json!({"tool_calls":[
                {"index":0,"id":&id1[10..],"type":"function","function":{"name":"ho","arguments":"{\"value\":"}},
                {"index":1,"id":id2,"type":"function","function":{"name":"echo","arguments":"{\"value\":"}}
            ]}),
            None,
        )
        + &frame(
            json!({"tool_calls":[{"index":1,"function":{"arguments":"2}"}},{"index":0,"function":{"arguments":"1}"}}]}),
            None,
        )
        + &frame(json!({}), Some("tool_calls"))
        + "data: [DONE]\n\n";
    let fixture = Fixture::start(vec![Reply::sse(first), Reply::sse(final_text("all done"))]).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut builder = ToolSet::builder();
    builder.register(Echo {
        spec: ToolSpec::new(
            "echo".parse().unwrap(),
            "Echo fixture",
            json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}),
        )
        .unwrap(),
        calls: calls.clone(),
    });
    let report = finish(start(provider(&fixture.endpoint), builder.build().unwrap())).await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    assert_eq!(report.requests, 2);
    assert_eq!(report.tool_rounds, 1);
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(
        assistant(&report, 0).finish_reason,
        ModelFinishReason::ToolCalls
    );
    let requests = fixture.finish().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["tools"][0]["function"]["name"], "echo");
    let messages = requests[1].body["messages"].as_array().unwrap();
    let previous = messages
        .iter()
        .find(|value| value["role"] == "assistant")
        .unwrap();
    assert_eq!(previous["reasoning_content"], "use tools");
    assert_eq!(previous["tool_calls"][0]["id"], id1);
    assert_eq!(previous["tool_calls"][1]["id"], id2);
    assert_eq!(
        previous["tool_calls"][0]["function"]["arguments"],
        "{\"value\":1}"
    );
    let results: Vec<_> = messages
        .iter()
        .filter(|value| value["role"] == "tool")
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["tool_call_id"], id1);
    assert_eq!(results[0]["content"], "echo:1");
    assert_eq!(results[1]["tool_call_id"], id2);
}

#[tokio::test]
async fn malformed_or_truncated_stream_never_becomes_success_or_retries() {
    let cases = vec![
        "data: [DONE]\n\n".to_owned(),
        "data: {invalid}\n\n".to_owned(),
        frame(json!({}), Some("stop")) + "data: [DONE]\n\n",
        frame(json!({"content":"partial"}), None),
        frame(json!({"content":"partial"}), Some("stop")),
        frame(json!({"content":"partial"}), None) + "data: {invalid}\n\n",
        frame(json!({"audio":{"id":"unsupported"}}), Some("stop")) + "data: [DONE]\n\n",
        frame(json!({"content":"partial"}), Some("stop"))
            + &frame(json!({}), Some("stop"))
            + "data: [DONE]\n\n",
    ];
    for body in cases {
        let fixture = Fixture::start(vec![Reply::sse(body)]).await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        let error = failure(&report);
        assert_eq!(*error.retry_hint(), RetryHint::Never);
        assert!(
            !report
                .appended
                .iter()
                .any(|item| matches!(item, HistoryItem::Assistant(_)))
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn runtime_is_the_only_retry_owner_and_only_recognized_rejections_retry() {
    let cases = [
        (
            429,
            json!({"error":{"code":"rate_limit_exceeded"}}),
            2,
            ModelErrorKind::RateLimited,
        ),
        (
            503,
            json!({"error":{"type":"overloaded_error"}}),
            2,
            ModelErrorKind::ProviderUnavailable,
        ),
        (
            429,
            json!({"error":{"code":"insufficient_quota","type":"rate_limit_error"}}),
            1,
            ModelErrorKind::QuotaExceeded,
        ),
        (
            429,
            json!({"error":{"message":"please retry"}}),
            1,
            ModelErrorKind::ProviderUnavailable,
        ),
        (
            503,
            json!({"error":{"message":"please retry"}}),
            1,
            ModelErrorKind::ProviderUnavailable,
        ),
        (
            500,
            json!({"error":{"type":"overloaded_error"}}),
            1,
            ModelErrorKind::ProviderUnavailable,
        ),
        (
            401,
            json!({"error":{"message":"fixture-not-a-real-key"}}),
            1,
            ModelErrorKind::AuthRejected,
        ),
    ];
    for (status, body, expected, kind) in cases {
        let fixture = Fixture::start(vec![
            Reply::error(status, body.to_string()),
            Reply::sse(final_text("retry result")),
        ])
        .await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        if expected == 2 {
            assert_eq!(report.outcome, LoopOutcome::Completed);
        } else {
            let error = failure(&report);
            assert_eq!(error.kind(), kind);
            assert_eq!(*error.retry_hint(), RetryHint::Never);
            assert!(
                !serde_json::to_string(error)
                    .unwrap()
                    .contains("fixture-not-a-real-key")
            );
        }
        assert_eq!(fixture.finish().await.len(), expected);
    }
}

#[tokio::test]
async fn a_model_start_alone_never_retries() {
    let fixture = Fixture::start(vec![
        Reply::error(
            429,
            json!({"error":{"code":"rate_limit_exceeded"}}).to_string(),
        ),
        Reply::sse(final_text("must not be called")),
    ])
    .await;
    let model = provider(&fixture.endpoint);
    let error = match model
        .start(request(), context(CancellationToken::new()))
        .await
    {
        Ok(_) => panic!("expected error"),
        Err(error) => error,
    };
    assert_eq!(error.delivery(), DeliveryState::NotStarted);
    assert!(matches!(error.retry_hint(), RetryHint::Retryable { .. }));
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn sse_error_retry_requires_no_prior_provider_choice_even_role_only() {
    let error_frame = format!(
        "data: {}\n\n",
        json!({"error":{"code":"rate_limit_exceeded"}})
    );
    for (prefix, expected) in [
        (String::new(), 2),
        (frame(json!({"role":"assistant"}), None), 1),
        (frame(json!({"content":"partial"}), None), 1),
    ] {
        let fixture = Fixture::start(vec![
            Reply::sse(prefix + &error_frame),
            Reply::sse(final_text("done")),
        ])
        .await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        if expected == 2 {
            assert_eq!(report.outcome, LoopOutcome::Completed);
        } else {
            assert_eq!(failure(&report).delivery(), DeliveryState::Started);
        }
        assert_eq!(fixture.finish().await.len(), expected);
    }
}

#[tokio::test]
async fn cancellation_after_partial_output_closes_transport_and_has_no_assistant_success() {
    let written = Arc::new(Notify::new());
    let disconnected = Arc::new(Notify::new());
    let fixture = Fixture::start(vec![Reply::Hold {
        prefix: frame(json!({"content":"partial before cancel"}), None),
        written: written.clone(),
        disconnected: disconnected.clone(),
    }])
    .await;
    let mut agent = start(provider(&fixture.endpoint), ToolSet::default());
    let handle = agent.handle();
    let mut events = agent.take_events().unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = events.recv().await {
            if matches!(event.event, LoopEvent::OutputDelta { .. }) {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(handle.cancel());
    let report = finish(agent).await;
    assert!(matches!(report.outcome, LoopOutcome::Cancelled(_)));
    assert!(report.partial.as_ref().unwrap().content.iter().any(|part| matches!(part, AssistantPart::Text(value) if value.as_ref() == "partial before cancel")));
    assert!(
        !report
            .appended
            .iter()
            .any(|item| matches!(item, HistoryItem::Assistant(_)))
    );
    tokio::time::timeout(Duration::from_secs(3), disconnected.notified())
        .await
        .unwrap();
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn cancellation_during_send_is_unknown_and_never_retried() {
    let received = Arc::new(Notify::new());
    let fixture = Fixture::start(vec![Reply::HeadersHold {
        received: received.clone(),
    }])
    .await;
    let model = provider(&fixture.endpoint);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task = tokio::spawn(async move { model.start(request(), context(token)).await });
    tokio::time::timeout(Duration::from_secs(3), received.notified())
        .await
        .unwrap();
    cancel.cancel();
    let error = match task.await.unwrap() {
        Ok(_) => panic!("expected cancellation"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ModelErrorKind::Cancelled);
    assert_eq!(error.delivery(), DeliveryState::Unknown);
    assert_eq!(*error.retry_hint(), RetryHint::Never);
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn explicit_deadline_stops_a_live_stream_without_completion() {
    let written = Arc::new(Notify::new());
    let disconnected = Arc::new(Notify::new());
    let fixture = Fixture::start(vec![Reply::Hold {
        prefix: frame(json!({"content":"partial"}), None),
        written,
        disconnected,
    }])
    .await;
    let model = provider(&fixture.endpoint);
    let mut context = context(CancellationToken::new());
    context.deadline = Instant::now() + Duration::from_secs(1);
    let mut stream = model.start(request(), context).await.unwrap();
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        ModelEvent::TextDelta { .. }
    ));
    let error = stream.next().await.unwrap().unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Timeout);
    assert_eq!(error.delivery(), DeliveryState::Started);
    assert_eq!(*error.retry_hint(), RetryHint::Never);
    drop(stream);
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn provider_finish_reasons_are_never_rewritten_to_stop() {
    for (raw, expected) in [
        ("length", ModelFinishReason::Length),
        ("content_filter", ModelFinishReason::ContentFiltered),
        ("refused", ModelFinishReason::Refused),
    ] {
        let fixture = Fixture::start(vec![Reply::sse(
            frame(json!({"content":"actual output"}), Some(raw)) + "data: [DONE]\n\n",
        )])
        .await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        let answer = assistant(&report, 0);
        assert_eq!(answer.finish_reason, expected);
        assert_eq!(
            answer.provider_replay.as_ref().unwrap().payload()["finish_reason"],
            raw
        );
        assert!(matches!(report.outcome, LoopOutcome::Failed(_)));
        assert_eq!(fixture.finish().await.len(), 1);
    }
    let fixture = Fixture::start(vec![Reply::sse(
        frame(
            json!({"content":"actual output"}),
            Some("provider_future_reason"),
        ) + "data: [DONE]\n\n",
    )])
    .await;
    let model = provider(&fixture.endpoint);
    let mut stream = model
        .start(request(), context(CancellationToken::new()))
        .await
        .unwrap();
    let mut finish_reason = None;
    let mut raw = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ModelEvent::Finish { reason } => finish_reason = Some(reason),
            ModelEvent::ProviderReplay { replay } => {
                raw = replay.payload()["finish_reason"]
                    .as_str()
                    .map(str::to_owned)
            }
            _ => {}
        }
    }
    assert_eq!(finish_reason, Some(ModelFinishReason::Unknown));
    assert_eq!(raw.as_deref(), Some("provider_future_reason"));
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn explicit_request_limits_and_reasoning_are_forwarded_without_defaults() {
    let fixture = Fixture::start(vec![Reply::sse(final_text("done"))]).await;
    let model = provider(&fixture.endpoint);
    let request = ModelRequest::new(
        vec![ModelMessage::user("test").unwrap()],
        vec![],
        ModelLimits::new(None, Some(123)).unwrap(),
        ReasoningPreference::High,
    )
    .unwrap();
    let mut stream = model
        .start(request, context(CancellationToken::new()))
        .await
        .unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    let requests = fixture.finish().await;
    assert_eq!(requests[0].body["max_completion_tokens"], 123);
    assert_eq!(requests[0].body["reasoning_effort"], "high");
}

#[tokio::test]
async fn redirects_are_not_followed_and_non_sse_success_is_not_completion() {
    let target = Fixture::start(vec![Reply::sse(final_text("redirect must not reach me"))]).await;
    let replies = vec![
        Reply::Full {
            status: 307,
            content_type: "text/plain",
            headers: format!("Location: {}\r\n", target.endpoint),
            body: String::new(),
            fragment: 0,
        },
        Reply::Full {
            status: 200,
            content_type: "application/json",
            headers: String::new(),
            body: "{}".to_owned(),
            fragment: 0,
        },
    ];
    for reply in replies {
        let fixture = Fixture::start(vec![reply]).await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        assert!(matches!(report.outcome, LoopOutcome::Failed(_)));
        assert_eq!(*failure(&report).retry_hint(), RetryHint::Never);
        assert_eq!(fixture.finish().await.len(), 1);
    }
    assert!(target.finish().await.is_empty());
}

#[tokio::test]
async fn long_retry_after_prevents_runtime_from_retrying_early() {
    let fixture = Fixture::start(vec![
        Reply::Full {
            status: 429,
            content_type: "application/json",
            headers: "Retry-After: 120\r\n".to_owned(),
            body: json!({"error":{"code":"rate_limit_exceeded"}}).to_string(),
            fragment: 0,
        },
        Reply::sse(final_text("must not happen")),
    ])
    .await;
    let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
    let error = failure(&report);
    assert_eq!(error.kind(), ModelErrorKind::RateLimited);
    assert!(
        matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(110))
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn missing_usage_is_unknown_and_inconsistent_usage_is_rejected() {
    let fixture = Fixture::start(vec![Reply::sse(final_text("no counts"))]).await;
    let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
    let usage = assistant(&report, 0).usage;
    assert_eq!(usage.input_tokens(), None);
    assert_eq!(usage.output_tokens(), None);
    assert_eq!(usage.reasoning_tokens(), None);
    assert_eq!(usage.provider_total_tokens(), None);
    fixture.finish().await;
    for usage in [
        json!({"prompt_tokens": 10,"completion_tokens": 2,"total_tokens": 99}),
        json!({"prompt_tokens": 10,"prompt_tokens_details":{"cached_tokens": 11}}),
        json!({"completion_tokens": 2,"completion_tokens_details":{"reasoning_tokens": 3}}),
        json!({"prompt_tokens": -1}),
    ] {
        let body = frame(json!({"content":"actual output"}), Some("stop"))
            + &format!("data: {}\n\n", json!({"choices":[], "usage":usage}))
            + "data: [DONE]\n\n";
        let fixture = Fixture::start(vec![Reply::sse(body)]).await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        assert_eq!(
            failure(&report).kind(),
            ModelErrorKind::InvalidProviderResponse
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn incomplete_tool_arguments_remain_partial_and_never_execute() {
    let fixture = Fixture::start(vec![Reply::sse(
        frame(json!({"tool_calls":[{"index":0,"id":"upstream_raw_id","type":"function","function":{"name":"echo","arguments":"{\"value\":"}}]}), None)
            + &frame(json!({}), Some("tool_calls")) + "data: [DONE]\n\n"
    )]).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut builder = ToolSet::builder();
    builder.register(Echo {
        spec: ToolSpec::new(
            "echo".parse().unwrap(),
            "Echo fixture",
            json!({"type":"object"}),
        )
        .unwrap(),
        calls: calls.clone(),
    });
    let report = finish(start(provider(&fixture.endpoint), builder.build().unwrap())).await;
    assert_eq!(
        failure(&report).kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert!(calls.lock().unwrap().is_empty());
    let partial = report.partial.as_ref().unwrap();
    assert_eq!(partial.incomplete_tool_calls.len(), 1);
    assert_eq!(
        partial.incomplete_tool_calls[0].tool_call_id.as_str(),
        "upstream_raw_id"
    );
    assert_eq!(
        partial.incomplete_tool_calls[0].arguments_text,
        "{\"value\":"
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn pre_cancelled_request_is_not_sent() {
    let fixture = Fixture::start(vec![Reply::sse(final_text("must not happen"))]).await;
    let model = provider(&fixture.endpoint);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = match model.start(request(), context(cancellation)).await {
        Ok(_) => panic!("expected cancellation"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ModelErrorKind::Cancelled);
    assert_eq!(error.delivery(), DeliveryState::NotStarted);
    assert_eq!(*error.retry_hint(), RetryHint::Never);
    assert!(fixture.finish().await.is_empty());
}

#[tokio::test]
async fn usage_reported_before_truncation_remains_authoritative_partial_usage() {
    let body = frame(json!({"content":"partial with counts"}), Some("stop"))
        + &format!(
            "data: {}\n\n",
            json!({"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}})
        );
    let fixture = Fixture::start(vec![Reply::sse(body)]).await;
    let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
    assert_eq!(failure(&report).kind(), ModelErrorKind::IncompleteResponse);
    assert_eq!(
        report
            .partial
            .as_ref()
            .unwrap()
            .usage
            .unwrap()
            .provider_total_tokens(),
        Some(7)
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn text_larger_than_one_runtime_event_is_split_on_unicode_boundaries() {
    let text = "🐈".repeat(20_000);
    let fixture = Fixture::start(vec![Reply::Full {
        status: 200,
        content_type: "text/event-stream",
        headers: String::new(),
        body: final_text(&text),
        fragment: 0,
    }])
    .await;
    let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    assert!(
        matches!(&assistant(&report, 0).content[0], AssistantPart::Text(value) if value.as_ref() == text)
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn unsupported_reasoning_history_is_rejected_before_network_io() {
    use minicore_runtime::model::ReasoningContent;
    let fixture = Fixture::start(vec![Reply::sse(final_text("must not happen"))]).await;
    let model = provider(&fixture.endpoint);
    let request = ModelRequest::new(
        vec![
            ModelMessage::assistant(vec![AssistantPart::Reasoning(
                ReasoningContent::new(
                    None,
                    None,
                    Some("opaque-encrypted-provider-state".to_owned()),
                    None,
                )
                .unwrap(),
            )])
            .unwrap(),
            ModelMessage::user("next").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    let error = match model
        .start(request, context(CancellationToken::new()))
        .await
    {
        Ok(_) => panic!("expected rejection"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ModelErrorKind::InvalidRequest);
    assert_eq!(error.delivery(), DeliveryState::NotStarted);
    assert!(fixture.finish().await.is_empty());
}

#[tokio::test]
async fn runtime_preserves_complete_unknown_finish_but_reports_failure() {
    let fixture = Fixture::start(vec![Reply::sse(
        frame(
            json!({"content":"complete provider output"}),
            Some("provider_future_reason"),
        ) + "data: [DONE]\n\n",
    )])
    .await;
    let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
    assert_eq!(
        failure(&report).kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    let answer = assistant(&report, 0);
    assert_eq!(answer.finish_reason, ModelFinishReason::Unknown);
    assert_eq!(
        answer.provider_replay.as_ref().unwrap().payload()["finish_reason"],
        "provider_future_reason"
    );
    assert!(report.partial.is_none());
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn truncated_tool_headers_are_preserved_as_non_executable_partial_metadata() {
    let fixture = Fixture::start(vec![Reply::sse(
        frame(json!({"tool_calls":[{"index":0,"id":"provider_id_fragment","type":"function","function":{"name":"ec"}}]}), None)
    )]).await;
    let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
    assert_eq!(failure(&report).kind(), ModelErrorKind::IncompleteResponse);
    assert_eq!(report.tool_rounds, 0);
    let partial = report.partial.as_ref().unwrap();
    assert!(partial.content.is_empty());
    assert!(partial.incomplete_tool_calls.is_empty());
    let replay = partial.provider_replay.as_ref().unwrap();
    assert_eq!(replay.format(), "minicore-host.openai-chat.partial.v1");
    assert_eq!(
        replay.payload()["incomplete_tool_headers"][0]["id_fragment"],
        "provider_id_fragment"
    );
    assert_eq!(
        replay.payload()["incomplete_tool_headers"][0]["name_fragment"],
        "ec"
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn unknown_tool_finish_does_not_authorize_tool_execution() {
    let fixture = Fixture::start(vec![Reply::sse(
        frame(json!({"tool_calls":[{"index":0,"id":"provider_id","type":"function","function":{"name":"echo","arguments":"{}"}}]}), None)
            + &frame(json!({}), Some("future_interrupted_reason")) + "data: [DONE]\n\n"
    )]).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut builder = ToolSet::builder();
    builder.register(Echo {
        spec: ToolSpec::new(
            "echo".parse().unwrap(),
            "Echo fixture",
            json!({"type":"object"}),
        )
        .unwrap(),
        calls: calls.clone(),
    });
    let report = finish(start(provider(&fixture.endpoint), builder.build().unwrap())).await;
    assert_eq!(
        failure(&report).kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        report
            .partial
            .as_ref()
            .unwrap()
            .provider_replay
            .as_ref()
            .unwrap()
            .payload()["finish_reason"],
        "future_interrupted_reason"
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn sse_long_retry_after_prevents_runtime_from_retrying_early() {
    let fixture = Fixture::start(vec![
        Reply::Full {
            status: 200,
            content_type: "text/event-stream",
            headers: "Retry-After: 60\r\n".to_owned(),
            body: format!("data: {}\n\n", json!({"error":{"type":"rate_limit_error"}})),
            fragment: 7,
        },
        Reply::sse(final_text("must not happen")),
    ])
    .await;
    let mut options = LoopOptions::default_checked().unwrap();
    options.model_retry_base_delay = Duration::from_millis(1);
    // A 60s delay fits this deadline but exceeds Runtime's 30s wait limit.
    // This must stop, not clamp the header or silently fall back to 1ms.
    options.model_timeout = Duration::from_secs(120);
    let report = finish(start_with_options(
        provider(&fixture.endpoint),
        ToolSet::default(),
        options,
    ))
    .await;
    let requests = fixture.finish().await;
    assert_eq!(
        requests.len(),
        1,
        "Retry-After: 60 must stop retries; request gaps: {:?}",
        requests
            .windows(2)
            .map(|pair| pair[1].received_at.duration_since(pair[0].received_at))
            .collect::<Vec<_>>()
    );
    let error = failure(&report);
    assert_eq!(error.kind(), ModelErrorKind::RateLimited);
    assert_eq!(error.delivery(), DeliveryState::NotStarted);
    assert!(
        matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(30))
    );
    assert!(error.diagnostic().message().as_ref().contains("HTTP 200"));
    assert_eq!(report.requests, 1);
    assert!(report.partial.is_none());
}

fn sse_rejection(headers: &str, error: Value, prefix: &str) -> Reply {
    Reply::Full {
        status: 200,
        content_type: "text/event-stream",
        headers: headers.to_owned(),
        body: format!("{prefix}data: {error}\n\n"),
        fragment: 7,
    }
}

// Test-only observation, not a replacement Model: every call delegates the
// unchanged request/context to the real provider and returns its exact result.
// Notification occurs in the same poll as Runtime receives the rejection; on
// the current-thread test executor the driver reaches its wait before the test
// wakes. This permits cancellation/time advancement without guessed sleeps.
struct TestObservedProvider {
    inner: Arc<OpenAiChatModel>,
    rejected: mpsc::UnboundedSender<ModelError>,
}
impl Model for TestObservedProvider {
    fn descriptor(&self) -> &ModelDescriptor {
        self.inner.descriptor()
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        Box::pin(async move {
            let result = self.inner.start(request, context).await;
            if let Err(error) = &result {
                let _ = self.rejected.send(error.clone());
            }
            result
        })
    }
}
fn observe_provider(endpoint: &str) -> (Arc<dyn Model>, mpsc::UnboundedReceiver<ModelError>) {
    let (rejected, receiver) = mpsc::unbounded_channel();
    (
        Arc::new(TestObservedProvider {
            inner: provider(endpoint),
            rejected,
        }),
        receiver,
    )
}
async fn rejection(receiver: &mut mpsc::UnboundedReceiver<ModelError>) -> ModelError {
    tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .expect("real HTTP provider did not return a rejection")
        .expect("provider observer ended")
}

#[tokio::test]
async fn sse_retry_after_and_exponential_backoff_are_actual_request_lower_bounds() {
    for (headers, base, minimum) in [
        ("Retry-After: 1\r\n", 1, 1_000),
        ("Retry-After-ms: 80\r\n", 1, 80),
        ("Retry-After-ms: NaN\r\nRetry-After: 0.08\r\n", 1, 80),
        ("Retry-After: 0.001\r\n", 80, 80),
    ] {
        let fixture = Fixture::start(vec![
            sse_rejection(
                headers,
                json!({"error":{"type":"overloaded_error"}}),
                ": keepalive\n\n",
            ),
            Reply::sse(final_text("retried once")),
        ])
        .await;
        let mut options = LoopOptions::default_checked().unwrap();
        options.model_retry_base_delay = Duration::from_millis(base);
        options.model_timeout = Duration::from_secs(3);
        let report = finish(start_with_options(
            provider(&fixture.endpoint),
            ToolSet::default(),
            options,
        ))
        .await;
        assert_eq!(report.outcome, LoopOutcome::Completed);
        assert_eq!(report.requests, 1, "retries are one logical model request");
        let requests = fixture.finish().await;
        assert_eq!(requests.len(), 2);
        let elapsed = requests[1]
            .received_at
            .duration_since(requests[0].received_at);
        assert!(
            elapsed >= Duration::from_millis(minimum),
            "{headers:?}: retry arrived after {elapsed:?}, before the {minimum}ms lower bound"
        );
        assert_eq!(requests[0].body, requests[1].body);
    }
}

#[tokio::test]
async fn sse_invalid_or_absent_retry_after_keeps_runtime_backoff() {
    for headers in [
        "",
        "Retry-After: -1\r\n",
        "Retry-After: NaN\r\n",
        "Retry-After: invalid\r\n",
        "Retry-After-ms: inf\r\n",
        "Retry-After: Thu, 01 Jan 1970 00:00:00 GMT\r\n",
    ] {
        let fixture = Fixture::start(vec![
            sse_rejection(headers, json!({"error":{"type":"rate_limit_error"}}), ""),
            Reply::sse(final_text("normal bounded retry")),
        ])
        .await;
        let (model, mut observed) = observe_provider(&fixture.endpoint);
        let mut options = LoopOptions::default_checked().unwrap();
        options.model_retry_base_delay = Duration::from_millis(40);
        options.model_timeout = Duration::from_secs(3);
        let agent = start_with_options(model, ToolSet::default(), options);
        let error = rejection(&mut observed).await;
        assert_eq!(error.kind(), ModelErrorKind::RateLimited);
        assert_eq!(error.delivery(), DeliveryState::NotStarted);
        assert_eq!(
            *error.retry_hint(),
            RetryHint::Retryable { retry_after: None }
        );
        assert!(error.diagnostic().message().as_ref().contains("HTTP 200"));
        assert_eq!(finish(agent).await.outcome, LoopOutcome::Completed);
        let requests = fixture.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1]
                .received_at
                .duration_since(requests[0].received_at)
                >= Duration::from_millis(40)
        );
    }
}

#[tokio::test]
async fn sse_retry_after_does_not_authorize_unknown_or_permanent_rejections() {
    let cases = [
        (
            json!({"error":{"type":"future_error","message":"rate_limit_error"}}),
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::Unknown,
        ),
        (
            json!({"error":{"type":"rate_limit_error","code":"insufficient_quota"}}),
            ModelErrorKind::QuotaExceeded,
            DeliveryState::NotStarted,
        ),
        (
            json!({"error":{"type":"authentication_error"}}),
            ModelErrorKind::AuthRejected,
            DeliveryState::NotStarted,
        ),
        (
            json!({"error":{"type":"invalid_request_error"}}),
            ModelErrorKind::InvalidRequest,
            DeliveryState::NotStarted,
        ),
    ];
    for (body, kind, delivery) in cases {
        let fixture = Fixture::start(vec![
            sse_rejection("Retry-After: 0\r\n", body, ""),
            Reply::sse(final_text("must not happen")),
        ])
        .await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        let error = failure(&report);
        assert_eq!(error.kind(), kind);
        assert_eq!(error.delivery(), delivery);
        assert_eq!(*error.retry_hint(), RetryHint::Never);
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn sse_retry_after_does_not_authorize_replay_after_role_or_text() {
    for (prefix, partial) in [
        (frame(json!({"role":"assistant"}), None), None),
        (
            frame(json!({"content":"preserve partial"}), None),
            Some("preserve partial"),
        ),
    ] {
        let fixture = Fixture::start(vec![
            sse_rejection(
                "Retry-After-ms: 1\r\n",
                json!({"error":{"type":"rate_limit_error"}}),
                &prefix,
            ),
            Reply::sse(final_text("must not happen")),
        ])
        .await;
        let report = finish(start(provider(&fixture.endpoint), ToolSet::default())).await;
        let error = failure(&report);
        assert_eq!(error.kind(), ModelErrorKind::RateLimited);
        assert_eq!(error.delivery(), DeliveryState::Started);
        assert_eq!(*error.retry_hint(), RetryHint::Never);
        if let Some(text) = partial {
            assert!(
                report.partial.as_ref().unwrap().content.iter().any(
                    |part| matches!(part, AssistantPart::Text(value) if value.as_ref() == text)
                )
            );
        }
        assert!(
            !report
                .appended
                .iter()
                .any(|item| matches!(item, HistoryItem::Assistant(_)))
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn sse_retry_after_must_fit_the_original_model_or_turn_deadline() {
    for turn_deadline in [false, true] {
        let fixture = Fixture::start(vec![
            sse_rejection(
                "Retry-After: 10\r\n",
                json!({"error":{"type":"rate_limit_error"}}),
                "",
            ),
            Reply::sse(final_text("must not happen")),
        ])
        .await;
        let mut options = LoopOptions::default_checked().unwrap();
        options.model_retry_base_delay = Duration::from_millis(1);
        if turn_deadline {
            options.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(3));
        } else {
            options.model_timeout = Duration::from_secs(3);
        }
        let report = finish(start_with_options(
            provider(&fixture.endpoint),
            ToolSet::default(),
            options,
        ))
        .await;
        let error = failure(&report);
        assert_eq!(error.kind(), ModelErrorKind::RateLimited);
        assert_eq!(error.delivery(), DeliveryState::NotStarted);
        assert!(
            matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(3))
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_interrupts_real_sse_retry_wait_without_another_request() {
    let fixture = Fixture::start(vec![
        sse_rejection(
            "Retry-After: 10\r\n",
            json!({"error":{"type":"rate_limit_error"}}),
            "",
        ),
        Reply::sse(final_text("must not happen")),
    ])
    .await;
    let (model, mut observed) = observe_provider(&fixture.endpoint);
    let mut options = LoopOptions::default_checked().unwrap();
    options.model_timeout = Duration::from_secs(60);
    options.model_retry_base_delay = Duration::from_millis(1);
    let agent = start_with_options(model, ToolSet::default(), options);
    let error = rejection(&mut observed).await;
    assert!(
        matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(3))
    );
    assert_eq!(fixture.request_count(), 1);
    assert!(agent.handle().cancel());
    assert_eq!(
        finish(agent).await.outcome,
        LoopOutcome::Cancelled(CancelReason::User)
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn model_timeout_during_real_sse_retry_wait_preserves_primary_rejection() {
    let fixture = Fixture::start(vec![
        sse_rejection(
            "Retry-After: 10\r\n",
            json!({"error":{"type":"rate_limit_error"}}),
            "",
        ),
        Reply::sse(final_text("must not happen")),
    ])
    .await;
    let (model, mut observed) = observe_provider(&fixture.endpoint);
    let mut options = LoopOptions::default_checked().unwrap();
    options.model_timeout = Duration::from_secs(20);
    options.model_retry_base_delay = Duration::from_millis(1);
    let agent = start_with_options(model, ToolSet::default(), options);
    let error = rejection(&mut observed).await;
    assert!(
        matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(3))
    );
    assert_eq!(fixture.request_count(), 1);
    // Real TCP has already delivered the rejection. Freeze only now so Tokio
    // cannot auto-advance time while that real network I/O is still pending.
    // Cross both timers: the original deadline wins over a late retry wakeup.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    let report = finish(agent).await;
    assert_eq!(failure(&report), &error);
    assert!(report.history_failure.is_none());
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn turn_deadline_during_real_sse_retry_wait_cancels_without_another_request() {
    let fixture = Fixture::start(vec![
        sse_rejection(
            "Retry-After: 10\r\n",
            json!({"error":{"type":"rate_limit_error"}}),
            "",
        ),
        Reply::sse(final_text("must not happen")),
    ])
    .await;
    let (model, mut observed) = observe_provider(&fixture.endpoint);
    let mut options = LoopOptions::default_checked().unwrap();
    options.model_timeout = Duration::from_secs(60);
    options.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(20));
    options.model_retry_base_delay = Duration::from_millis(1);
    let agent = start_with_options(model, ToolSet::default(), options);
    let error = rejection(&mut observed).await;
    assert!(
        matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(3))
    );
    assert_eq!(fixture.request_count(), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    assert_eq!(
        finish(agent).await.outcome,
        LoopOutcome::Cancelled(CancelReason::Deadline)
    );
    assert_eq!(fixture.finish().await.len(), 1);
}
