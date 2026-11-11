//! Real TLS, production HTTP/SSE resource clients and Runtime execution. The
//! local scripted provider and credential source are fixtures, not a live model.
mod os_pipe;
mod reload_common;
mod wss_common;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use reload_common::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, mpsc},
    task::JoinHandle,
};

struct HttpFixture {
    endpoint: String,
    requests: mpsc::UnboundedReceiver<Value>,
    task: JoinHandle<()>,
}
impl HttpFixture {
    async fn start(text: &'static str, gate: Option<Arc<Semaphore>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (send, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let boundary = loop {
                if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break index + 4;
                }
                let mut chunk = [0; 4096];
                let count = socket.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&chunk[..count]);
            };
            let head = String::from_utf8(bytes[..boundary].to_vec()).unwrap();
            let length: usize = head
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < boundary + length {
                let mut chunk = [0; 4096];
                let count = socket.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&chunk[..count]);
            }
            send.send(serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap())
                .unwrap();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            let body = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
            );
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }
    async fn request(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn subscription(client: &HostClient) -> SubscribeRequest {
    SubscribeRequest {
        scope: SubscriptionScope::Host {
            host_id: client.initial_info().host_id.clone(),
        },
        categories: [EventCategory::State].into_iter().collect(),
    }
}
async fn ended(subscription: &mut ClientSubscription) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match subscription.recv().await {
                Err(CallError::Connection { .. }) | Ok(None) => return,
                Ok(Some(_)) => {}
                other => panic!("unexpected stream result {other:?}"),
            }
        }
    })
    .await
    .unwrap();
}
fn input(text: &str) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: text.into(),
        in_reply_to: None,
    }
}
async fn turn_id(session: &SessionClient, index: usize) -> TurnId {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let turns = session.turns(options()).await.unwrap().data;
            if turns.len() > index {
                return turns[index].turn_id.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn canonical_wss_rotation_keeps_an_accepted_http_turn_and_next_turn_binds_new_resources() {
    let finish_a = Arc::new(Semaphore::new(0));
    let mut provider_a =
        HttpFixture::start("A finished after rotation", Some(finish_a.clone())).await;
    let mut provider_b = HttpFixture::start("B finished", None).await;
    let mut original = config();
    original.framework_instructions = "FRAMEWORK A".into();
    original.models = vec![OpenAiModelSettings {
        model: "fixture/chat".parse().unwrap(),
        endpoint: provider_a.endpoint.clone(),
        provider_model: "provider-A".into(),
        api_key_env: "TASK035_MODEL".into(),
        context_window: 64_000,
        supported_reasoning: vec![ReasoningPreference::Auto],
        supports_tools: true,
        request_timeout_ms: None,
    }];
    let (host, preparer) = CredentialFixture::host(original.clone(), false).await;
    let server = tls_server(host.clone()).await;
    let operator = server.client(KEY_A).await.unwrap();
    let network_reader = server.client(KEY_A).await.unwrap();
    let mut raw_reader = server.raw().await;
    assert_eq!(
        raw_reader.initialize(KEY_A).await["result"]["can_operate"],
        false
    );
    let local = HostClient::in_process(host.clone()).unwrap();
    let (pipe, pipe_server) = stdio(host).await;
    let mut events = network_reader
        .subscribe(subscription(&network_reader), options())
        .await
        .unwrap();
    let before = operator.info(options()).await.unwrap().control;
    let mut agent = AgentConfig::default();
    agent.session_defaults.model = "fixture/chat".parse().unwrap();
    agent.session_defaults.execution.compaction.enabled = false;
    agent.role.instructions = "ROLE A".into();
    let created = operator
        .create_agent(
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "Actual HTTP turn".into(),
                mode: StorageMode::Memory,
                config: agent,
                default_session: None,
                initial_message: None,
            },
            options(),
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    let session = operator.session(target.clone());
    session.send(input("first task"), options()).await.unwrap();
    let first_request = provider_a.request().await;
    let first_turn = turn_id(&session, 0).await;
    let mut same_key = original.clone();
    same_key.name = "Changed name, same actual key".into();
    same_key.network.access_key_env = Some(SOURCE_ALIAS_A.into());
    let unchanged = operator
        .reload(request(same_key.clone(), 0), options())
        .await
        .unwrap();
    assert_eq!(
        terminal(&operator, unchanged.operation_id).await.result,
        HostReloadResult::Applied { config_revision: 1 }
    );
    assert_eq!(operator.info(options()).await.unwrap().control, before);
    // The original subscription is still owned, not silently recreated.
    assert!(
        network_reader
            .unsubscribe(
                UnsubscribeRequest {
                    subscription_id: events.result().subscription_id.clone()
                },
                options()
            )
            .await
            .unwrap()
            .ended
    );
    drop(events);
    events = network_reader
        .subscribe(subscription(&network_reader), options())
        .await
        .unwrap();
    let mut candidate = same_key;
    candidate.framework_instructions = "FRAMEWORK B".into();
    candidate.models[0].endpoint = provider_b.endpoint.clone();
    candidate.models[0].provider_model = "provider-B".into();
    candidate.network.access_key_env = Some(SOURCE_B.into());
    let reload = request(candidate.clone(), 1);
    let operation = reload.operation_id.clone();
    // The accepted owner may invalidate this WSS connection before reply delivery.
    match operator.reload(reload, options()).await {
        Ok(receipt) => assert_eq!(
            receipt.result,
            HostReloadResult::Preparing { config_revision: 1 }
        ),
        Err(CallError::Connection { .. }) => {}
        other => panic!("unexpected reload result: {other:?}"),
    }
    assert_eq!(
        terminal(&local, operation.clone()).await.result,
        HostReloadResult::Applied { config_revision: 2 }
    );
    raw_reader.assert_disconnected().await;
    ended(&mut events).await;
    assert!(matches!(
        operator.info(options()).await,
        Err(CallError::Connection { .. })
    ));
    assert!(matches!(
        network_reader.info(options()).await,
        Err(CallError::Connection { .. })
    ));
    assert_eq!(
        fault(server.client(KEY_A).await.err().unwrap()).kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    // Existing trusted readers survive but cannot inherit the revoked operator.
    for reader in [&local, &pipe] {
        assert_eq!(reader.config(options()).await.unwrap().config, candidate);
        assert!(!reader.info(options()).await.unwrap().control.occupied);
        assert_eq!(
            fault(
                reader
                    .reload(request(original.clone(), 2), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostAccessReadOnly)
        );
    }
    let replacement = server.client(KEY_B).await.unwrap();
    assert!(replacement.initial_info().can_operate);
    assert_eq!(
        terminal(&replacement, operation).await.result,
        HostReloadResult::Applied { config_revision: 2 }
    );
    assert_eq!(preparer.count(), 2);
    finish_a.add_permits(1);
    let session = replacement.session(target);
    let first_result = session.wait_turn(first_turn, options()).await.unwrap();
    assert_eq!(
        first_result.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(
        serde_json::to_string(&first_result.data.history)
            .unwrap()
            .contains("A finished after rotation")
    );
    session.send(input("next task"), options()).await.unwrap();
    let second_request = provider_b.request().await;
    let second_turn = turn_id(&session, 1).await;
    let second_result = session.wait_turn(second_turn, options()).await.unwrap();
    assert_eq!(
        second_result.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(
        serde_json::to_string(&second_result.data.history)
            .unwrap()
            .contains("B finished")
    );
    for (body, model, framework) in [
        (&first_request, "provider-A", "FRAMEWORK A"),
        (&second_request, "provider-B", "FRAMEWORK B"),
    ] {
        assert_eq!(body["model"], model);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["content"] == framework)
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["content"] == "ROLE A")
                .count(),
            1
        );
    }
    assert!(
        second_request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["content"] == "A finished after rotation")
    );
    let public = serde_json::to_string(&replacement.config(options()).await.unwrap()).unwrap();
    for key in [KEY_A, KEY_B, MODEL_KEY] {
        assert!(!public.contains(key));
    }
    pipe.close();
    pipe_server.await.unwrap().unwrap();
}

#[tokio::test]
async fn canonical_key_rotation_preserves_typed_and_stdio_operation_rights() {
    for transport in 0..2 {
        let (host, _) = CredentialFixture::host(config(), false).await;
        let operator = Connected::new(host.clone(), transport).await;
        let server = tls_server(host).await;
        let observer = server.client(KEY_A).await.unwrap();
        let mut events = observer
            .subscribe(subscription(&observer), options())
            .await
            .unwrap();
        let before = operator.client.info(options()).await.unwrap().control;
        let mut candidate = config();
        candidate.network.access_key_env = Some(SOURCE_B.into());
        let receipt = operator
            .client
            .reload(request(candidate, 0), options())
            .await
            .unwrap();
        assert_eq!(
            terminal(&operator.client, receipt.operation_id)
                .await
                .result,
            HostReloadResult::Applied { config_revision: 1 }
        );
        ended(&mut events).await;
        assert_eq!(
            operator.client.info(options()).await.unwrap().control,
            before
        );
        let replacement = server.client(KEY_B).await.unwrap();
        assert!(!replacement.initial_info().can_operate);
        assert_eq!(
            fault(
                replacement
                    .reload(request(config(), 1), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostAccessReadOnly)
        );
        operator.finish().await;
    }
}
