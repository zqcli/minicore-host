mod support;

use futures_util::StreamExt;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_runtime::{
    DeadlineSource, ExecutionConfig, ExecutionOptions, InteractionId, LoopId, ToolResultHistory,
    interaction::{InteractionAnswer, PendingInteraction},
    model::{
        AssistantPart, DeliveryState, Model, ModelCallContext, ModelCallEvent, ModelCallParams,
        ModelCallReport, ModelDescriptor, ModelDriver, ModelDriverConfig, ModelError,
        ModelErrorKind, ModelEvent, ModelFinishReason, ModelLimits, ModelMessage, ModelPartial,
        ModelRequest, ModelResponse, ModelStartFuture, ReasoningPreference, RetryHint,
    },
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolBatchContext, ToolBatchEvent, ToolBatchHost, ToolBatchStop, ToolContext,
        ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet, ToolSpec,
        run_tool_batch,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use support::{Fixture, Reply};
use tokio::sync::{Notify, mpsc, oneshot};
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
fn call_options() -> ExecutionOptions {
    ExecutionOptions {
        model_retry_base_delay: Duration::from_millis(1),
        model_timeout: Duration::from_secs(3),
        ..ExecutionOptions::default()
    }
}
fn full_request(tools: &ToolSet) -> ModelRequest {
    ModelRequest::new(
        vec![
            ModelMessage::system("Framework and role").unwrap(),
            ModelMessage::user("fixture user request").unwrap(),
        ],
        tools.specs().to_vec(),
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap()
}
async fn call(model: Arc<dyn Model>, tools: ToolSet) -> ModelCallReport {
    tokio::time::timeout(
        Duration::from_secs(5),
        call_with_options(model, tools, call_options()),
    )
    .await
    .unwrap()
}
async fn call_with_options(
    model: Arc<dyn Model>,
    tools: ToolSet,
    options: ExecutionOptions,
) -> ModelCallReport {
    call_cancelable(model, tools, options, CancellationToken::new()).await
}
async fn call_cancelable(
    model: Arc<dyn Model>,
    tools: ToolSet,
    options: ExecutionOptions,
    cancellation: CancellationToken,
) -> ModelCallReport {
    let driver = ModelDriver::new(
        model.as_ref(),
        ModelDriverConfig::new(
            options.model_timeout,
            options.model_retry_attempts,
            options.model_retry_base_delay,
        ),
    )
    .unwrap();
    driver
        .call(
            full_request(&tools),
            ModelCallParams {
                loop_id: LoopId::new().unwrap(),
                request_index: 0,
                cancellation,
                deadline: options.deadline.map(Into::into),
            },
            &mut |_| {},
        )
        .await
}
async fn join_call(task: tokio::task::JoinHandle<ModelCallReport>) -> ModelCallReport {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
}
fn assistant(report: &ModelCallReport) -> &ModelResponse {
    report.outcome.as_ref().unwrap()
}
fn failure(report: &ModelCallReport) -> &ModelError {
    report.outcome.as_ref().unwrap_err().error()
}
fn model_partial(report: &ModelCallReport) -> Option<&ModelPartial> {
    report
        .outcome
        .as_ref()
        .err()
        .and_then(|failure| failure.partial.as_deref())
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
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert!(report.outcome.is_ok());
    let answer = assistant(&report);
    assert_eq!(answer.finish_reason(), ModelFinishReason::Stop);
    assert!(
        matches!(&answer.parts()[0], AssistantPart::Reasoning(value) if value.text() == Some("think 🐈"))
    );
    assert!(
        matches!(&answer.parts()[1], AssistantPart::Text(value) if value.as_ref() == "hello 世界")
    );
    assert_eq!(answer.usage().unwrap().input_tokens(), Some(10));
    assert_eq!(answer.usage().unwrap().output_tokens(), Some(6));
    assert_eq!(answer.usage().unwrap().reasoning_tokens(), Some(3));
    assert_eq!(answer.usage().unwrap().cache_read_tokens(), Some(2));
    assert_eq!(answer.usage().unwrap().provider_total_tokens(), Some(21));
    assert_eq!(answer.provider_replay().unwrap().payload()["usage"], usage);
    assert!(
        !serde_json::to_string(assistant(&report))
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

struct NoInteractionFixture;
impl ToolBatchHost for NoInteractionFixture {
    fn observe(&self, _: ToolBatchEvent) {}
    fn request_interaction(
        &self,
        _: PendingInteraction,
        _: oneshot::Sender<InteractionAnswer>,
    ) -> Result<(), ToolBatchStop> {
        panic!("Echo fixture must not request interaction")
    }
    fn finish_interaction(&self, _: InteractionId) {
        panic!("Echo fixture has no interaction to finish")
    }
}

// A fixed two-request HTTP exchange, not a standalone scheduling loop.
async fn tool_exchange(
    model: Arc<dyn Model>,
    tools: ToolSet,
) -> (ModelResponse, Vec<ToolResultHistory>, ModelResponse) {
    let options = call_options();
    let driver = ModelDriver::new(
        model.as_ref(),
        ModelDriverConfig::new(
            options.model_timeout,
            options.model_retry_attempts,
            options.model_retry_base_delay,
        ),
    )
    .unwrap();
    let initial = full_request(&tools);
    let context = ModelCallParams {
        loop_id: LoopId::new().unwrap(),
        request_index: 0,
        cancellation: CancellationToken::new(),
        deadline: Some(Instant::now() + Duration::from_secs(3)),
    };
    let first = driver
        .call(initial.clone(), context.clone(), &mut |_| {})
        .await
        .outcome
        .unwrap();
    assert_eq!(first.finish_reason(), ModelFinishReason::ToolCalls);
    let execution = ExecutionConfig::new(
        model.clone(),
        ReasoningPreference::Auto,
        tools,
        None,
        Arc::new(DefaultPromptProvider::new(None)),
    )
    .unwrap();
    let calls: Vec<_> = first
        .parts()
        .iter()
        .filter_map(AssistantPart::as_tool_call)
        .collect();
    let batch = run_tool_batch(
        &execution,
        &calls,
        ToolBatchContext {
            loop_id: context.loop_id,
            request_index: context.request_index,
            cancellation: context.cancellation.clone(),
            deadline: context.deadline,
            policy_timeout: Duration::from_secs(3),
            tool_timeout: Duration::from_secs(3),
        },
        Arc::new(NoInteractionFixture),
    )
    .await;
    assert!(batch.stop.is_none());
    let mut messages = initial.messages().to_vec();
    messages.push(
        ModelMessage::assistant_with_provider_replay(
            first.parts().to_vec(),
            first.provider_replay().cloned(),
        )
        .unwrap(),
    );
    messages.extend(batch.results.iter().map(|result| {
        ModelMessage::tool_with_outcome(
            result.call_id.clone(),
            result.output.clone(),
            result.outcome,
        )
        .unwrap()
    }));
    let next = ModelRequest::new(
        messages,
        initial.tools().to_vec(),
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    let mut next_context = context;
    next_context.request_index = 1;
    let final_response = driver
        .call(next, next_context, &mut |_| {})
        .await
        .outcome
        .unwrap();
    assert_eq!(final_response.finish_reason(), ModelFinishReason::Stop);
    (first, batch.results, final_response)
}

#[tokio::test]
async fn model_and_tool_primitives_preserve_original_ids_and_history() {
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
    let (first, results, _) =
        tool_exchange(provider(&fixture.endpoint), builder.build().unwrap()).await;
    assert_eq!(first.finish_reason(), ModelFinishReason::ToolCalls);
    assert_eq!(results.len(), 2);
    assert_eq!(calls.lock().unwrap().len(), 2);
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
        let error = failure(&report);
        assert_eq!(*error.retry_hint(), RetryHint::Never);
        assert!(report.outcome.is_err());
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
        if expected == 2 {
            assert!(report.outcome.is_ok());
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
        if expected == 2 {
            assert!(report.outcome.is_ok());
        } else {
            assert_eq!(failure(&report).delivery(), DeliveryState::Started);
        }
        assert_eq!(fixture.finish().await.len(), expected);
    }
}

#[tokio::test]
async fn typed_stream_decoding_preserves_structured_error_priority() {
    for (payload, kind, delivery, retryable) in [
        (
            r#"{"choices":"bad","error":{"code":"rate_limit_exceeded"}}"#,
            ModelErrorKind::RateLimited,
            DeliveryState::NotStarted,
            true,
        ),
        (
            r#"{"error":{"type":"overloaded_error"},"choices":[{"delta":false}]}"#,
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::NotStarted,
            true,
        ),
        (
            r#"{"error":{"type":"rate_limit_error","code":"insufficient_quota"},"choices":null}"#,
            ModelErrorKind::QuotaExceeded,
            DeliveryState::NotStarted,
            false,
        ),
        (
            r#"{"error":{"error":{"code":"rate_limit_exceeded"}}}"#,
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::Unknown,
            false,
        ),
        (
            r#"{"error":false,"choices":"bad"}"#,
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::Unknown,
            false,
        ),
        (
            r#"{"error":null,"choices":"bad"}"#,
            ModelErrorKind::InvalidProviderResponse,
            DeliveryState::Unknown,
            false,
        ),
    ] {
        let fixture = Fixture::start(vec![Reply::Full {
            status: 200,
            content_type: "text/event-stream",
            headers: "Retry-After-ms: 2500\r\n".to_owned(),
            body: format!("data: {payload}\n\n"),
            fragment: 7,
        }])
        .await;
        let error = match provider(&fixture.endpoint)
            .start(request(), context(CancellationToken::new()))
            .await
        {
            Ok(_) => panic!("expected rejection for {payload}"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), kind, "{payload}");
        assert_eq!(error.delivery(), delivery, "{payload}");
        if retryable {
            assert!(
                matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(1)),
                "{payload}"
            );
        } else {
            assert_eq!(*error.retry_hint(), RetryHint::Never, "{payload}");
        }
        assert_eq!(fixture.finish().await.len(), 1);
    }

    // A present but null error is an ordinary frame; a non-null error still
    // wins when all normal fields could have decoded successfully.
    let payload = r#"{"error":null,"choices":[{"index":0,"delta":{"content":"kept"},"finish_reason":"stop"}]}"#;
    let fixture = Fixture::start(vec![Reply::sse(format!(
        "data: {payload}\n\ndata: [DONE]\n\n"
    ))])
    .await;
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert!(
        matches!(&assistant(&report).parts()[0], AssistantPart::Text(text) if text.as_ref() == "kept")
    );
    assert_eq!(fixture.finish().await.len(), 1);

    let payload = r#"{"error":{"code":"insufficient_quota"},"choices":[{"index":0,"delta":{"content":"must not escape"}}]}"#;
    let fixture = Fixture::start(vec![Reply::sse(format!("data: {payload}\n\n"))]).await;
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert_eq!(failure(&report).kind(), ModelErrorKind::QuotaExceeded);
    assert_eq!(failure(&report).delivery(), DeliveryState::NotStarted);
    assert!(model_partial(&report).is_none());
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn typed_stream_decoding_rejects_duplicate_recognized_fields() {
    for payload in [
        r#"{"id":"a","id":"b","choices":[{"index":0,"delta":{"content":"bad"}}]}"#,
        r#"{"choices":[],"choices":[{"index":0,"delta":{"content":"bad"}}]}"#,
        r#"{"choices":[{"index":0,"index":0,"delta":{"content":"bad"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"content":"first","content":"second"}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","function":{"name":"echo","arguments":"{}","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[],"usage":{"total_tokens":1,"total_tokens":2}}"#,
        r#"{"error":null,"error":{"code":"rate_limit_exceeded"},"choices":[]}"#,
        r#"{"id":"a","id":"b","error":{"code":"rate_limit_exceeded"}}"#,
    ] {
        let fixture = Fixture::start(vec![Reply::sse(format!("data: {payload}\n\n"))]).await;
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
        assert_eq!(
            failure(&report).kind(),
            ModelErrorKind::InvalidProviderResponse,
            "{payload}"
        );
        assert_eq!(*failure(&report).retry_hint(), RetryHint::Never);
        assert!(model_partial(&report).is_none());
        assert_eq!(fixture.finish().await.len(), 1);
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
    let model = provider(&fixture.endpoint);
    let cancellation = CancellationToken::new();
    let token = cancellation.clone();
    let (events, mut observed) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let driver = ModelDriver::new(
            model.as_ref(),
            ModelDriverConfig::new(Duration::from_secs(3), 2, Duration::from_millis(1)),
        )
        .unwrap();
        driver
            .call(
                full_request(&ToolSet::default()),
                ModelCallParams {
                    loop_id: LoopId::new().unwrap(),
                    request_index: 0,
                    cancellation: token,
                    deadline: None,
                },
                &mut |event| {
                    let _ = events.send(event);
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = observed.recv().await {
            if matches!(event, ModelCallEvent::TextDelta(_)) {
                break;
            }
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    let report = join_call(task).await;
    assert_eq!(failure(&report).kind(), ModelErrorKind::Cancelled);
    assert!(model_partial(&report).unwrap().content.iter().any(|part| matches!(part, AssistantPart::Text(value) if value.as_ref() == "partial before cancel")));
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
        let answer = assistant(&report);
        assert_eq!(answer.finish_reason(), expected);
        assert_eq!(
            answer.provider_replay().unwrap().payload()["finish_reason"],
            raw
        );
        assert!(report.outcome.is_ok());
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
        assert!(report.outcome.is_err());
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
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
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
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert!(assistant(&report).usage().is_none());
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
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
    let report = call(provider(&fixture.endpoint), builder.build().unwrap()).await;
    assert_eq!(
        failure(&report).kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert!(calls.lock().unwrap().is_empty());
    let partial = model_partial(&report).unwrap();
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
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert_eq!(failure(&report).kind(), ModelErrorKind::IncompleteResponse);
    assert_eq!(
        model_partial(&report)
            .unwrap()
            .usage
            .unwrap()
            .provider_total_tokens(),
        Some(7)
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn unicode_text_above_the_former_event_limit_is_preserved() {
    let text = "🐈".repeat(20_000);
    let fixture = Fixture::start(vec![Reply::Full {
        status: 200,
        content_type: "text/event-stream",
        headers: String::new(),
        body: final_text(&text),
        fragment: 0,
    }])
    .await;
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert!(report.outcome.is_ok());
    assert!(
        matches!(&assistant(&report).parts()[0], AssistantPart::Text(value) if value.as_ref() == text)
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn unsupported_reasoning_history_is_rejected_before_network_io() {
    use minicore_host_runtime::model::ReasoningContent;
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
async fn model_driver_preserves_complete_unknown_finish_for_the_owner() {
    let fixture = Fixture::start(vec![Reply::sse(
        frame(
            json!({"content":"complete provider output"}),
            Some("provider_future_reason"),
        ) + "data: [DONE]\n\n",
    )])
    .await;
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert!(report.outcome.is_ok());
    let answer = assistant(&report);
    assert_eq!(answer.finish_reason(), ModelFinishReason::Unknown);
    assert_eq!(
        answer.provider_replay().unwrap().payload()["finish_reason"],
        "provider_future_reason"
    );
    assert!(model_partial(&report).is_none());
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn truncated_tool_headers_are_preserved_as_non_executable_partial_metadata() {
    let fixture = Fixture::start(vec![Reply::sse(
        frame(json!({"tool_calls":[{"index":0,"id":"provider_id_fragment","type":"function","function":{"name":"ec"}}]}), None)
    )]).await;
    let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
    assert_eq!(failure(&report).kind(), ModelErrorKind::IncompleteResponse);
    assert!(report.outcome.is_err());
    let partial = model_partial(&report).unwrap();
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
    let report = call(provider(&fixture.endpoint), builder.build().unwrap()).await;
    assert_eq!(
        failure(&report).kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        model_partial(&report)
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
    // A 60s delay fits this deadline but exceeds Runtime's 30s wait limit.
    // This must stop, not clamp the header or silently fall back to 1ms.
    let options = ExecutionOptions {
        model_retry_base_delay: Duration::from_millis(1),
        model_timeout: Duration::from_secs(120),
        ..ExecutionOptions::default()
    };
    let report = call_with_options(provider(&fixture.endpoint), ToolSet::default(), options).await;
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
    assert_eq!(report.attempts, 1);
    assert!(model_partial(&report).is_none());
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
        let options = ExecutionOptions {
            model_retry_base_delay: Duration::from_millis(base),
            model_timeout: Duration::from_secs(3),
            ..ExecutionOptions::default()
        };
        let report =
            call_with_options(provider(&fixture.endpoint), ToolSet::default(), options).await;
        assert!(report.outcome.is_ok());
        assert_eq!(report.attempts, 2);
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
        let options = ExecutionOptions {
            model_retry_base_delay: Duration::from_millis(40),
            model_timeout: Duration::from_secs(3),
            ..ExecutionOptions::default()
        };
        let task = tokio::spawn(call_with_options(model, ToolSet::default(), options));
        let error = rejection(&mut observed).await;
        assert_eq!(error.kind(), ModelErrorKind::RateLimited);
        assert_eq!(error.delivery(), DeliveryState::NotStarted);
        assert_eq!(
            *error.retry_hint(),
            RetryHint::Retryable { retry_after: None }
        );
        assert!(error.diagnostic().message().as_ref().contains("HTTP 200"));
        assert!(join_call(task).await.outcome.is_ok());
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
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
        let report = call(provider(&fixture.endpoint), ToolSet::default()).await;
        let error = failure(&report);
        assert_eq!(error.kind(), ModelErrorKind::RateLimited);
        assert_eq!(error.delivery(), DeliveryState::Started);
        assert_eq!(*error.retry_hint(), RetryHint::Never);
        if let Some(text) = partial {
            assert!(
                model_partial(&report).unwrap().content.iter().any(
                    |part| matches!(part, AssistantPart::Text(value) if value.as_ref() == text)
                )
            );
        }
        assert!(report.outcome.is_err());
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
        let options = ExecutionOptions {
            model_retry_base_delay: Duration::from_millis(1),
            model_timeout: Duration::from_secs(if turn_deadline { 600 } else { 3 }),
            deadline: turn_deadline.then(|| tokio::time::Instant::now() + Duration::from_secs(3)),
            ..ExecutionOptions::default()
        };
        let report =
            call_with_options(provider(&fixture.endpoint), ToolSet::default(), options).await;
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
    let options = ExecutionOptions {
        model_timeout: Duration::from_secs(60),
        model_retry_base_delay: Duration::from_millis(1),
        ..ExecutionOptions::default()
    };
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(call_cancelable(
        model,
        ToolSet::default(),
        options,
        cancellation.clone(),
    ));
    let error = rejection(&mut observed).await;
    assert!(
        matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(3))
    );
    // Prove that the real driver remains in the header-directed wait before
    // cancelling it, without a wall-clock sleep or another network attempt.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(fixture.request_count(), 1);
    cancellation.cancel();
    let report = join_call(task).await;
    assert!(cancellation.is_cancelled());
    assert_eq!(failure(&report), &error);
    assert_eq!(report.attempts, 1);
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
    let options = ExecutionOptions {
        model_timeout: Duration::from_secs(20),
        model_retry_base_delay: Duration::from_millis(1),
        ..ExecutionOptions::default()
    };
    let task = tokio::spawn(call_with_options(model, ToolSet::default(), options));
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
    let report = join_call(task).await;
    assert_eq!(failure(&report), &error);
    assert_eq!(report.attempts, 1);
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
    let options = ExecutionOptions {
        model_timeout: Duration::from_secs(60),
        deadline: Some(tokio::time::Instant::now() + Duration::from_secs(20)),
        model_retry_base_delay: Duration::from_millis(1),
        ..ExecutionOptions::default()
    };
    let task = tokio::spawn(call_with_options(model, ToolSet::default(), options));
    let error = rejection(&mut observed).await;
    assert!(
        matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(3))
    );
    assert_eq!(fixture.request_count(), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    let report = join_call(task).await;
    let failure = report.outcome.as_ref().unwrap_err();
    assert_eq!(failure.error(), &error);
    assert_eq!(failure.deadline_source(), Some(DeadlineSource::Turn));
    assert_eq!(report.attempts, 1);
    assert_eq!(fixture.finish().await.len(), 1);
}

fn payload_sse(body: String) -> Reply {
    Reply::Full {
        status: 200,
        content_type: "text/event-stream",
        headers: String::new(),
        body,
        // Exercise network fragmentation independently of provider delta size.
        fragment: 4093,
    }
}

fn echo_fixture(calls: Arc<Mutex<Vec<ToolInvocation>>>) -> ToolSet {
    let mut builder = ToolSet::builder();
    builder.register(Echo {
        spec: ToolSpec::new(
            "echo".parse().unwrap(),
            "Echo fixture",
            json!({"type":"object"}),
        )
        .unwrap(),
        calls,
    });
    builder.build().unwrap()
}

#[tokio::test]
async fn large_text_and_reasoning_deltas_reach_runtime_without_adapter_splitting() {
    let text = "猫🐈".repeat(40_000);
    let reasoning = "思考\t🦉".repeat(40_000);
    assert!(text.len() > 262_144 && reasoning.len() > 262_144);
    let body = frame(json!({"reasoning_content":reasoning}), None)
        + &frame(json!({"content":text}), None)
        + &frame(json!({}), Some("stop"))
        + "data: [DONE]\n\n";
    let fixture = Fixture::start(vec![payload_sse(body.clone()), payload_sse(body)]).await;
    let model = provider(&fixture.endpoint);
    let mut stream = model
        .start(request(), context(CancellationToken::new()))
        .await
        .unwrap();
    let mut text_deltas = Vec::new();
    let mut reasoning_deltas = Vec::new();
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ModelEvent::TextDelta { delta } => text_deltas.push(delta.into_string()),
            ModelEvent::ReasoningDelta { delta } => reasoning_deltas.push(delta.into_string()),
            _ => {}
        }
    }
    assert_eq!(text_deltas.as_slice(), std::slice::from_ref(&text));
    assert_eq!(
        reasoning_deltas.as_slice(),
        std::slice::from_ref(&reasoning)
    );

    let report = call(model, ToolSet::default()).await;
    assert!(report.outcome.is_ok());
    assert!(model_partial(&report).is_none());
    let content = &assistant(&report).parts();
    assert_eq!(content.len(), 2);
    assert!(
        matches!(&content[0], AssistantPart::Reasoning(value) if value.text() == Some(reasoning.as_str()))
    );
    assert!(matches!(&content[1], AssistantPart::Text(value) if value.as_ref() == text));
    assert_eq!(fixture.finish().await.len(), 2);
}

#[tokio::test]
async fn large_tool_arguments_and_results_execute_once_and_replay_exactly() {
    let value = "工具 🐈\n".repeat(30_000);
    let arguments = json!({"value":value,"wide":vec![0; 5_000]}).to_string();
    assert!(arguments.len() > 262_144);
    let mut split = arguments.len() / 2;
    while !arguments.is_char_boundary(split) {
        split -= 1;
    }
    let first = frame(
        json!({"tool_calls":[{"index":0,"id":"large-original-id","type":"function","function":{"name":"echo","arguments":&arguments[..split]}}]}),
        None,
    ) + &frame(
        json!({"tool_calls":[{"index":0,"function":{"arguments":&arguments[split..]}}]}),
        None,
    ) + &frame(json!({}), Some("tool_calls"))
        + "data: [DONE]\n\n";
    let fixture = Fixture::start(vec![
        payload_sse(first),
        Reply::sse(final_text("large tool completed")),
    ])
    .await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (_, results, _) =
        tool_exchange(provider(&fixture.endpoint), echo_fixture(calls.clone())).await;
    let invocations = calls.lock().unwrap().clone();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].arguments["value"], value);
    assert_eq!(
        invocations[0].arguments["wide"].as_array().unwrap().len(),
        5_000
    );
    let expected_output = format!("echo:{}", json!(value));
    assert!(expected_output.len() > 262_144);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].call_id.as_str(), "large-original-id");
    assert_eq!(results[0].output.content().as_str(), expected_output);
    let requests = fixture.finish().await;
    assert_eq!(requests.len(), 2);
    let messages = requests[1].body["messages"].as_array().unwrap();
    let previous = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .unwrap();
    assert_eq!(previous["tool_calls"][0]["id"], "large-original-id");
    assert_eq!(
        previous["tool_calls"][0]["function"]["arguments"],
        arguments
    );
    let tool_result = messages
        .iter()
        .find(|message| message["role"] == "tool")
        .unwrap();
    assert_eq!(tool_result["tool_call_id"], "large-original-id");
    assert_eq!(tool_result["content"], expected_output);
}

#[tokio::test]
async fn invalid_large_or_deep_tool_arguments_remain_exact_nonexecutable_partials() {
    let large_incomplete = format!("{{\"value\":\"{}", "未完🐈".repeat(40_000));
    assert!(large_incomplete.len() > 262_144);
    let too_deep = format!("{{\"value\":{}0{}}}", "[".repeat(127), "]".repeat(127));
    for arguments in [large_incomplete, too_deep, "[]".to_owned()] {
        let body = frame(
            json!({"tool_calls":[{"index":0,"id":"invalid-arguments-id","type":"function","function":{"name":"echo","arguments":arguments}}]}),
            None,
        ) + &frame(json!({}), Some("tool_calls"))
            + "data: [DONE]\n\n";
        let fixture = Fixture::start(vec![payload_sse(body)]).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let report = call(provider(&fixture.endpoint), echo_fixture(calls.clone())).await;
        assert_eq!(
            failure(&report).kind(),
            ModelErrorKind::InvalidProviderResponse
        );
        assert_eq!(*failure(&report).retry_hint(), RetryHint::Never);
        assert!(calls.lock().unwrap().is_empty());
        assert!(report.outcome.is_err());
        let partial = model_partial(&report).unwrap();
        assert_eq!(partial.incomplete_tool_calls.len(), 1);
        assert_eq!(
            partial.incomplete_tool_calls[0].tool_call_id.as_str(),
            "invalid-arguments-id"
        );
        assert_eq!(partial.incomplete_tool_calls[0].arguments_text, arguments);
        assert_eq!(fixture.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn raw_tool_arguments_reach_runtime_without_text_character_repair() {
    for arguments in [
        "{\"value\":\"a\0b\"}",
        "{\"value\":\"a\u{1b}b\"}",
        "{\"value\":\"a\rb\"}",
    ] {
        let body = frame(
            json!({"tool_calls":[{"index":0,"id":"raw-call","function":{"name":"echo","arguments":arguments}}]}),
            None,
        ) + &frame(json!({}), Some("tool_calls"))
            + "data: [DONE]\n\n";
        let fixture = Fixture::start(vec![Reply::sse(body)]).await;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let report = call(provider(&fixture.endpoint), echo_fixture(calls.clone())).await;
        assert_eq!(
            failure(&report).kind(),
            ModelErrorKind::InvalidProviderResponse
        );
        assert_eq!(failure(&report).delivery(), DeliveryState::Started);
        assert_eq!(*failure(&report).retry_hint(), RetryHint::Never);
        assert!(calls.lock().unwrap().is_empty());
        let partial = model_partial(&report).unwrap();
        assert_eq!(partial.incomplete_tool_calls[0].arguments_text, arguments);
        assert_eq!(
            partial.provider_replay.as_ref().unwrap().format(),
            "minicore-host.openai-chat.v1"
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }

    let arguments = "{\r\n\"value\":\"\\u0000\\u001b\u{7f}\u{85}\"}";
    let body = frame(
        json!({"tool_calls":[{"index":0,"id":"valid-controls","function":{"name":"echo","arguments":arguments}}]}),
        None,
    ) + &frame(json!({}), Some("tool_calls"))
        + "data: [DONE]\n\n";
    let fixture = Fixture::start(vec![Reply::sse(body)]).await;
    let report = call(
        provider(&fixture.endpoint),
        echo_fixture(Arc::new(Mutex::new(Vec::new()))),
    )
    .await;
    let call = assistant(&report).parts()[0].as_tool_call().unwrap();
    assert_eq!(call.arguments(), &json!({"value":"\0\u{1b}\u{7f}\u{85}"}));
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn later_invalid_tool_preserves_verified_call_and_protocol_metadata() {
    let bad_arguments = "{\"value\":\"unfinished";
    let usage = json!({"prompt_tokens":5,"completion_tokens":3,"total_tokens":8});
    let body = frame(
        json!({"tool_calls":[
            {"index":0,"id":"verified-call","function":{"name":"echo","arguments":"{\"value\":1}"}},
            {"index":1,"id":"invalid-call","function":{"name":"echo","arguments":bad_arguments}}
        ]}),
        None,
    ) + &frame(json!({}), Some("tool_calls"))
        + &format!("data: {}\n\n", json!({"choices":[],"usage":usage}))
        + "data: [DONE]\n\n";
    let fixture = Fixture::start(vec![Reply::sse(body)]).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let report = call(provider(&fixture.endpoint), echo_fixture(calls.clone())).await;
    assert_eq!(
        failure(&report).kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert_eq!(*failure(&report).retry_hint(), RetryHint::Never);
    assert!(calls.lock().unwrap().is_empty());
    let partial = model_partial(&report).unwrap();
    assert_eq!(partial.content.len(), 1);
    let verified = partial.content[0].as_tool_call().unwrap();
    assert_eq!(verified.tool_call_id().as_str(), "verified-call");
    assert_eq!(verified.call_index(), 0);
    assert_eq!(verified.arguments(), &json!({"value":1}));
    assert_eq!(partial.incomplete_tool_calls.len(), 1);
    assert_eq!(
        partial.incomplete_tool_calls[0].tool_call_id.as_str(),
        "invalid-call"
    );
    assert_eq!(partial.incomplete_tool_calls[0].call_index, 1);
    assert_eq!(partial.incomplete_tool_calls[0].part_index, 1);
    assert_eq!(
        partial.incomplete_tool_calls[0].arguments_text,
        bad_arguments
    );
    assert_eq!(partial.usage.unwrap().provider_total_tokens(), Some(8));
    let replay = partial.provider_replay.as_ref().unwrap();
    assert_eq!(replay.format(), "minicore-host.openai-chat.v1");
    assert_eq!(
        replay.payload(),
        &json!({"finish_reason":"tool_calls","usage":usage})
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn large_incomplete_tool_header_payload_keeps_raw_provider_fragments() {
    let arguments = format!("{{\"value\":\"{}", "raw 🐈".repeat(40_000));
    assert!(arguments.len() > 262_144);
    let body = frame(
        json!({"tool_calls":[{"index":0,"id":"","type":"function","function":{"name":"echo","arguments":arguments}}]}),
        None,
    );
    let fixture = Fixture::start(vec![payload_sse(body)]).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let report = call(provider(&fixture.endpoint), echo_fixture(calls.clone())).await;
    assert_eq!(
        failure(&report).kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert!(calls.lock().unwrap().is_empty());
    assert!(report.outcome.is_err());
    let partial = model_partial(&report).unwrap();
    assert!(partial.content.is_empty());
    assert!(partial.incomplete_tool_calls.is_empty());
    let headers = &partial.provider_replay.as_ref().unwrap().payload()["incomplete_tool_headers"];
    assert_eq!(headers[0]["id_fragment"], "");
    assert_eq!(headers[0]["name_fragment"], "echo");
    assert_eq!(headers[0]["arguments_fragment"], arguments);
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn tool_json_above_the_old_nesting_limit_retains_parser_stack_safety() {
    let arguments = format!("{{\"value\":{}0{}}}", "[".repeat(64), "]".repeat(64));
    let fixture = Fixture::start(vec![
        payload_sse(
            frame(
                json!({"tool_calls":[{"index":0,"id":"nested-call","type":"function","function":{"name":"echo","arguments":arguments}}]}),
                None,
            ) + &frame(json!({}), Some("tool_calls")) + "data: [DONE]\n\n",
        ),
        Reply::sse(final_text("nested tool completed")),
    ])
    .await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let _ = tool_exchange(provider(&fixture.endpoint), echo_fixture(calls.clone())).await;
    assert_eq!(calls.lock().unwrap().len(), 1);
    let requests = fixture.finish().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].body["messages"][2]["tool_calls"][0]["function"]["arguments"],
        arguments
    );
}

#[tokio::test]
async fn large_http_error_bodies_keep_structured_classification_and_retry_headers() {
    let private_message = "private-provider-body-🐈".repeat(15_000);
    for (status, code, expected_kind, attempts) in [
        (429, "rate_limit_exceeded", ModelErrorKind::RateLimited, 2),
        (
            503,
            "overloaded_error",
            ModelErrorKind::ProviderUnavailable,
            2,
        ),
        (429, "insufficient_quota", ModelErrorKind::QuotaExceeded, 1),
        (429, "future_error", ModelErrorKind::ProviderUnavailable, 1),
    ] {
        let body = json!({"error":{"code":code,"message":private_message}}).to_string();
        assert!(body.len() > 262_144);
        let fixture = Fixture::start(vec![
            Reply::Full {
                status,
                content_type: "application/json",
                headers: "Retry-After-ms: 80\r\n".to_owned(),
                body,
                fragment: 4093,
            },
            Reply::sse(final_text("after recognized rejection")),
        ])
        .await;
        let (model, mut rejections) = observe_provider(&fixture.endpoint);
        let report = call(model, ToolSet::default()).await;
        let error = rejection(&mut rejections).await;
        assert_eq!(error.kind(), expected_kind);
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private-provider-body")
        );
        let requests = fixture.finish().await;
        assert_eq!(requests.len(), attempts);
        if attempts == 2 {
            assert!(report.outcome.is_ok());
            assert_eq!(error.delivery(), DeliveryState::NotStarted);
            assert!(matches!(error.retry_hint(), RetryHint::Retryable { .. }));
            assert!(
                requests[1]
                    .received_at
                    .duration_since(requests[0].received_at)
                    >= Duration::from_millis(80)
            );
        } else {
            assert_eq!(failure(&report).kind(), expected_kind);
            assert_eq!(*error.retry_hint(), RetryHint::Never);
            assert_eq!(
                error.delivery(),
                if code == "future_error" {
                    DeliveryState::Unknown
                } else {
                    DeliveryState::NotStarted
                }
            );
        }
    }
}

#[tokio::test]
async fn large_http_errors_do_not_relax_retry_wait_limits_or_parse_invalid_json() {
    let padding = "body-only-diagnostic-🐈".repeat(15_000);
    let valid = json!({"error":{"code":"rate_limit_exceeded","message":padding}}).to_string();
    let invalid = format!("{{\"error\":{{\"code\":\"rate_limit_exceeded\",\"message\":\"{padding}");
    for (body, valid) in [(valid, true), (invalid, false)] {
        assert!(body.len() > 262_144);
        let fixture = Fixture::start(vec![
            Reply::Full {
                status: 429,
                content_type: "application/json",
                headers: "Retry-After: 60\r\n".to_owned(),
                body,
                fragment: 4093,
            },
            Reply::sse(final_text("must not happen")),
        ])
        .await;
        let options = ExecutionOptions {
            model_timeout: Duration::from_secs(120),
            model_retry_base_delay: Duration::from_millis(1),
            ..ExecutionOptions::default()
        };
        let report =
            call_with_options(provider(&fixture.endpoint), ToolSet::default(), options).await;
        let error = failure(&report);
        if valid {
            assert_eq!(error.kind(), ModelErrorKind::RateLimited);
            assert!(
                matches!(error.retry_hint(), RetryHint::Retryable { retry_after: Some(delay) } if *delay > Duration::from_secs(30))
            );
        } else {
            assert_eq!(error.delivery(), DeliveryState::Unknown);
            assert_eq!(*error.retry_hint(), RetryHint::Never);
        }
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("body-only-diagnostic")
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}
