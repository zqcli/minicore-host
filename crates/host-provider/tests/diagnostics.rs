//! New reconstruction regression: actual Runtime and loopback HTTP/SSE, not a
//! live provider, fabricated business events or a subscriber-dependent owner.
#[allow(dead_code)]
mod support;
use futures_util::StreamExt;
use minicore_host_diagnostics::{Context, LoggingState, Options, Output, ShutdownOutcome};
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_runtime::{
    LoopId,
    model::{
        DeliveryState, Model, ModelCallContext, ModelCallParams, ModelCallReport, ModelDescriptor,
        ModelDriver, ModelDriverConfig, ModelErrorKind, ModelEvent, ModelLimits, ModelMessage,
        ModelRequest, ReasoningPreference,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use support::{Fixture, Reply};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct LogFile(PathBuf);
impl Drop for LogFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn provider(endpoint: &str) -> Arc<OpenAiChatModel> {
    let descriptor = ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([ReasoningPreference::Auto]),
        true,
    )
    .unwrap();
    Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(endpoint, "PRIVATE_PROVIDER_MODEL", descriptor),
            SecretApiKey::new("PRIVATE_PROVIDER_KEY").unwrap(),
        )
        .unwrap(),
    )
}
fn frame(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"PRIVATE_UPSTREAM_ID", "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}
fn response(usage: bool) -> String {
    let mut body = frame(json!({"reasoning_content":"PRIVATE_REASONING"}), None)
        + &frame(json!({"content":"PRIVATE_MODEL_BODY"}), None)
        + &frame(json!({}), Some("stop"));
    if usage {
        body += &format!(
            "data: {}\n\n",
            json!({"choices":[],"usage":{
                "prompt_tokens":12,"completion_tokens":9,"total_tokens":21,
                "prompt_tokens_details":{"cached_tokens":2},
                "completion_tokens_details":{"reasoning_tokens":3}
            }})
        );
    }
    body + "data: [DONE]\n\n"
}
async fn run(model: Arc<dyn Model>) -> (LoopId, ModelCallReport) {
    let driver = ModelDriver::new(
        model.as_ref(),
        ModelDriverConfig::new(Duration::from_secs(3), 2, Duration::from_millis(1)),
    )
    .unwrap();
    let request = ModelRequest::new(
        vec![
            ModelMessage::system("PRIVATE_ROLE").unwrap(),
            ModelMessage::user("PRIVATE_USER_INPUT").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    let context = ModelCallParams {
        loop_id: LoopId::new().unwrap(),
        request_index: 0,
        cancellation: CancellationToken::new(),
        deadline: None,
    };
    let id = context.loop_id;
    let report = tokio::time::timeout(
        Duration::from_secs(5),
        driver.call(request, context, &mut |_| {}),
    )
    .await
    .unwrap();
    (id, report)
}
fn context(index: u32, cancellation: CancellationToken) -> ModelCallContext {
    ModelCallContext::new(
        LoopId::new().unwrap(),
        index,
        cancellation,
        Instant::now() + Duration::from_secs(3),
    )
}
fn request() -> ModelRequest {
    ModelRequest::new(
        vec![ModelMessage::user("PRIVATE_USER_INPUT").unwrap()],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap()
}
fn finished(rows: &[Value], id: LoopId) -> Vec<&Value> {
    rows.iter()
        .filter(|row| {
            row["event"] == "model.transport_finished"
                && row["attributes"]["loop_id"] == id.to_string()
        })
        .collect()
}

#[tokio::test]
async fn actual_provider_facts_keep_runtime_ids_privacy_and_harmless_output_lifecycle() {
    // No subscriber: actual work still completes.
    let disabled = Fixture::start(vec![Reply::sse(response(false))]).await;
    let (disabled_id, disabled_report) = run(provider(&disabled.endpoint)).await;
    assert!(disabled_report.outcome.is_ok());
    assert_eq!(disabled.finish().await.len(), 1);

    let log = LogFile(std::env::temp_dir().join(format!(
        "minicore-provider-diagnostics-{}.jsonl",
        LoopId::new().unwrap()
    )));
    let mut options = Options::new("recovered-provider-diagnostics");
    options.level = tracing::Level::TRACE;
    options.output = Output::File(log.0.clone());
    let guard = minicore_host_diagnostics::install(options).unwrap();
    let health = guard.health();

    let retry = Fixture::start(vec![
        Reply::error(
            429,
            json!({"error":{"code":"rate_limit_exceeded","message":"PRIVATE_ERROR_BODY"}})
                .to_string(),
        ),
        Reply::sse(response(true)),
    ])
    .await;
    let endpoint = retry.endpoint.clone();
    let model = provider(&endpoint);
    let unpolled = context(99, CancellationToken::new());
    let unpolled_id = unpolled.loop_id;
    drop(model.start(request(), unpolled));
    assert_eq!(retry.request_count(), 0);
    let (request_id, report) = run(model).await;
    assert!(report.outcome.is_ok());
    assert_eq!(report.attempts, 2);
    let answer = report.outcome.as_ref().unwrap();
    assert_eq!(answer.usage().unwrap().input_tokens(), Some(10));
    assert_eq!(answer.usage().unwrap().output_tokens(), Some(6));
    assert_eq!(retry.finish().await.len(), 2);

    let rejected = Fixture::start(vec![Reply::error(
        401,
        "PRIVATE_PROVIDER_KEY PRIVATE_ERROR_BODY",
    )])
    .await;
    let rejected_model = provider(&rejected.endpoint);
    let rejected_context = context(7, CancellationToken::new());
    let rejected_id = rejected_context.loop_id;
    // Deliberately unrelated parent-span metadata must not become local identity.
    let unrelated = Context {
        host_incarnation: Some("00000000000000000001".parse().unwrap()),
        ..Default::default()
    };
    let error = match unrelated
        .instrument(rejected_model.start(request(), rejected_context))
        .await
    {
        Ok(_) => panic!("expected actual auth rejection"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ModelErrorKind::AuthRejected);
    assert_eq!(error.delivery(), DeliveryState::NotStarted);
    assert_eq!(rejected.finish().await.len(), 1);

    let unknown = Fixture::start(vec![Reply::sse(response(false))]).await;
    let unknown_context = context(8, CancellationToken::new());
    let unknown_id = unknown_context.loop_id;
    let mut stream = provider(&unknown.endpoint)
        .start(request(), unknown_context)
        .await
        .unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    drop(stream);
    assert_eq!(unknown.finish().await.len(), 1);

    let written = Arc::new(Notify::new());
    let disconnected = Arc::new(Notify::new());
    let dropped = Fixture::start(vec![Reply::Hold {
        prefix: frame(json!({"content":"PRIVATE_PARTIAL"}), None),
        written,
        disconnected: disconnected.clone(),
    }])
    .await;
    let dropped_context = context(9, CancellationToken::new());
    let dropped_id = dropped_context.loop_id;
    let mut stream = provider(&dropped.endpoint)
        .start(request(), dropped_context)
        .await
        .unwrap();
    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        ModelEvent::TextDelta { .. }
    ));
    drop(stream);
    tokio::time::timeout(Duration::from_secs(3), disconnected.notified())
        .await
        .unwrap();
    assert_eq!(dropped.finish().await.len(), 1);

    let received = Arc::new(Notify::new());
    let cancelled = Fixture::start(vec![Reply::HeadersHold {
        received: received.clone(),
    }])
    .await;
    let model = provider(&cancelled.endpoint);
    let cancellation = CancellationToken::new();
    let cancel_context = context(10, cancellation.clone());
    let cancelled_id = cancel_context.loop_id;
    let task = tokio::spawn(async move { model.start(request(), cancel_context).await });
    tokio::time::timeout(Duration::from_secs(3), received.notified())
        .await
        .unwrap();
    cancellation.cancel();
    let error = match task.await.unwrap() {
        Ok(_) => panic!("expected cancellation"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ModelErrorKind::Cancelled);
    assert_eq!(error.delivery(), DeliveryState::Unknown);
    assert_eq!(cancelled.finish().await.len(), 1);

    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log.0).unwrap();
    for forbidden in [
        "PRIVATE_PROVIDER_KEY",
        "PRIVATE_PROVIDER_MODEL",
        "PRIVATE_UPSTREAM_ID",
        "PRIVATE_USER_INPUT",
        "PRIVATE_ROLE",
        "PRIVATE_MODEL_BODY",
        "PRIVATE_REASONING",
        "PRIVATE_PARTIAL",
        "PRIVATE_ERROR_BODY",
        "00000000000000000001",
        &endpoint,
    ] {
        assert!(
            !text.contains(forbidden),
            "private data leaked: {forbidden}"
        );
    }
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 12);
    for row in &rows {
        assert_eq!(row["schema_version"], 1);
        assert_eq!(row["scope"], "RUNTIME");
        assert_eq!(row["level"], "DEBUG");
        assert_eq!(row["component"], "provider.http");
        for field in [
            "host_id",
            "host_incarnation",
            "agent_id",
            "session_id",
            "turn_id",
            "tool_call_id",
            "message_id",
        ] {
            assert!(row.get(field).is_none(), "invented source {field}");
        }
        assert_ne!(row["attributes"]["loop_id"], unpolled_id.to_string());
        assert_ne!(row["attributes"]["loop_id"], disabled_id.to_string());
        if row["event"] == "model.transport_finished" {
            assert!(row["attributes"]["duration_ms"].as_u64().is_some());
        }
    }
    let attempts = finished(&rows, request_id);
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0]["attributes"]["upstream_status"], 429);
    assert_eq!(attempts[0]["attributes"]["error_kind"], "rate_limited");
    assert_eq!(attempts[0]["attributes"]["delivery"], "not_started");
    assert_eq!(attempts[1]["attributes"]["upstream_status"], 200);
    assert_eq!(attempts[1]["attributes"]["request_index"], 0);
    for (name, expected) in [
        ("input_tokens", 10),
        ("output_tokens", 6),
        ("reasoning_tokens", 3),
        ("cache_read_tokens", 2),
        ("provider_total_tokens", 21),
    ] {
        assert_eq!(attempts[1]["attributes"][name], expected);
    }
    assert!(
        attempts[1]["attributes"]
            .get("cache_write_tokens")
            .is_none()
    );
    let rejected = finished(&rows, rejected_id);
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0]["attributes"]["request_index"], 7);
    assert_eq!(rejected[0]["attributes"]["error_kind"], "auth_rejected");
    let unknown = finished(&rows, unknown_id);
    assert_eq!(unknown.len(), 1);
    for name in ["input_tokens", "output_tokens", "provider_total_tokens"] {
        assert!(unknown[0]["attributes"].get(name).is_none());
    }
    let dropped = finished(&rows, dropped_id);
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0]["attributes"]["outcome"], "dropped");
    assert!(dropped[0]["attributes"].get("error_kind").is_none());
    let cancelled = finished(&rows, cancelled_id);
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0]["attributes"]["error_kind"], "cancelled");
    assert_eq!(cancelled[0]["attributes"]["delivery"], "unknown");
    assert!(cancelled[0]["attributes"].get("upstream_status").is_none());

    // Closed output cannot fail or resubmit a subsequent actual Runtime call.
    let after = Fixture::start(vec![Reply::sse(response(false))]).await;
    assert!(run(provider(&after.endpoint)).await.1.outcome.is_ok());
    assert_eq!(after.finish().await.len(), 1);
    assert_eq!(health.snapshot().status, LoggingState::Degraded);
    assert!(health.snapshot().dropped_records.unwrap() > 0);
    assert_eq!(std::fs::read_to_string(&log.0).unwrap(), text);
}
