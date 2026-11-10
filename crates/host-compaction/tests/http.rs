//! Actual Host provider + Runtime Model types over loopback TCP and SSE.
//! Responses are fixtures, not live-model or public Session integration claims.
use minicore_host_compaction::{HistorySnapshot, generate, prepare};
use minicore_host_protocol::CompactionSettings;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_runtime::{
    HistoryItem, LoopId, UserHistory, UserInput, UserMessageKind,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelLimits, ModelMessage, ModelRequest,
        ReasoningPreference,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};
use tokio_util::sync::CancellationToken;

fn model(endpoint: &str) -> OpenAiChatModel {
    let descriptor = ModelDescriptor::new(
        "fixture/summary".parse().unwrap(),
        1000,
        BTreeSet::from([ReasoningPreference::Auto]),
        true,
    )
    .unwrap();
    OpenAiChatModel::new(
        OpenAiChatConfig::new(endpoint, "fixture-summary", descriptor),
        SecretApiKey::new("fixture-only-not-a-real-secret").unwrap(),
    )
    .unwrap()
}
fn plan(model: &dyn Model) -> minicore_host_compaction::CompactionPlan {
    let old = "Retain the actual project facts. ".repeat(300);
    let user = |text: &str| {
        HistoryItem::User(UserHistory {
            loop_id: LoopId::new().unwrap(),
            kind: UserMessageKind::Prompt,
            input: UserInput::text(text).unwrap(),
        })
    };
    let source = HistorySnapshot {
        revision: 3,
        items: vec![user(&old), user("Current instruction")].into(),
    };
    let request = ModelRequest::new(
        vec![
            ModelMessage::system("Framework and role").unwrap(),
            ModelMessage::user(old).unwrap(),
            ModelMessage::user("Current instruction").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    prepare(
        source,
        1,
        model.descriptor(),
        &CompactionSettings::default(),
        request,
    )
    .unwrap()
    .unwrap()
}
fn context() -> ModelCallContext {
    ModelCallContext::new(
        LoopId::new().unwrap(),
        0,
        CancellationToken::new(),
        Instant::now() + Duration::from_secs(5),
    )
}
fn frame(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"fixture-summary-response","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}
#[path = "../../../tests/support/http.rs"]
mod http;

#[tokio::test]
async fn actual_http_sse_summary_preserves_optional_usage_and_has_no_tools() {
    for reported in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = http::read_request(&mut socket).await.body;
            let mut body = frame(json!({"content":"Actual summary from SSE."}), None)
                + &frame(json!({}), Some("stop"));
            if reported {
                body += &format!(
                    "data: {}\n\n",
                    json!({"choices":[],"usage":{"prompt_tokens":31,"completion_tokens":7,"total_tokens":38}})
                );
            }
            body += "data: [DONE]\n\n";
            socket
                .write_all(http::sse_headers(&body).as_bytes())
                .await
                .unwrap();
            // Actual fragmented SSE passes through the provider parser.
            for chunk in body.as_bytes().chunks(7) {
                socket.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
            request
        });
        let model = model(&endpoint);
        let candidate = generate(&plan(&model), &model, context()).await.unwrap();
        let sent = server.await.unwrap();
        assert!(sent.get("tools").is_none());
        assert!(
            sent.get("max_completion_tokens").is_none(),
            "soft target is not an injected hard output cap"
        );
        assert_eq!(sent["messages"][0]["role"], "system");
        assert_eq!(sent["messages"][1]["role"], "user");
        assert_eq!(
            candidate.summary.content.as_str(),
            "Actual summary from SSE."
        );
        if reported {
            assert_eq!(
                candidate.utility_usage[0].usage.unwrap().input_tokens(),
                None
            );
            assert_eq!(
                candidate.utility_usage[0]
                    .usage
                    .unwrap()
                    .provider_total_tokens(),
                Some(38)
            );
        } else {
            assert_eq!(candidate.utility_usage[0].usage, None);
        }
    }
}

#[tokio::test]
async fn actual_http_cancellation_closes_the_stream_without_a_candidate() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let written = Arc::new(Notify::new());
    let server_written = written.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        http::read_request(&mut socket).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        socket
            .write_all(frame(json!({"content":"unfinished summary"}), None).as_bytes())
            .await
            .unwrap();
        server_written.notify_one();
        let mut buffer = [0; 1];
        let closed = socket.read(&mut buffer).await;
        assert!(
            matches!(closed, Ok(0) | Err(_)),
            "actual provider connection must close"
        );
    });
    let model = model(&endpoint);
    let plan = plan(&model);
    let context = context();
    let cancellation = context.cancellation.clone();
    let worker = tokio::spawn(async move { generate(&plan, &model, context).await });
    written.notified().await;
    cancellation.cancel();
    let result = worker.await.unwrap().unwrap_err();
    assert_eq!(
        result.error,
        minicore_host_compaction::CompactionError::Cancelled
    );
    assert!(!result.utility_usage[0].completed);
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
