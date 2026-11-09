#![allow(dead_code)]
//! Production Host, Runtime, resource assembler and HTTP/SSE client. Only the
//! loopback HTTP responses, response gates and credentials are test fixtures.
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_transport::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
};

pub const KEY: &str = "task040-loopback-only-access-key";
pub const SUMMARY: &str = "Retain the original task and its recorded results.";

pub fn options() -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(5_000),
    }
}
pub fn compact() -> CompactRequest {
    CompactRequest {
        operation_id: OperationId::new().unwrap(),
    }
}
pub fn cancel(target_operation_id: OperationId) -> CompactCancelRequest {
    CompactCancelRequest {
        operation_id: OperationId::new().unwrap(),
        target_operation_id,
    }
}
pub fn control() -> SessionControlRequest {
    SessionControlRequest {
        operation_id: OperationId::new().unwrap(),
    }
}
pub fn input(text: &str, mode: MessageMode) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode,
        text: text.into(),
        in_reply_to: None,
    }
}
pub fn fault(error: CallError) -> Fault {
    match error {
        CallError::Business(fault) => fault,
        other => panic!("expected business Fault, got {other:?}"),
    }
}
pub fn reply(text: &str) -> (String, Option<Arc<Semaphore>>) {
    (
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
        ),
        None,
    )
}
pub fn gated(text: &str, gate: &Arc<Semaphore>) -> (String, Option<Arc<Semaphore>>) {
    (reply(text).0, Some(gate.clone()))
}
pub fn seed_replies() -> Vec<(String, Option<Arc<Semaphore>>)> {
    vec![
        reply(&"recorded answer ".repeat(150)),
        reply(&"second answer ".repeat(170)),
    ]
}
pub struct HttpFixture {
    pub endpoint: String,
    requests: mpsc::UnboundedReceiver<Value>,
    task: JoinHandle<()>,
}
impl HttpFixture {
    pub async fn start(replies: Vec<(String, Option<Arc<Semaphore>>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (send, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut workers = JoinSet::new();
            for (body, gate) in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let send = send.clone();
                workers.spawn(async move {
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
                    let length: usize = head.lines().find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                    }).unwrap();
                    while bytes.len() < boundary + length {
                        let mut chunk = [0; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert_ne!(count, 0);
                        bytes.extend_from_slice(&chunk[..count]);
                    }
                    send.send(serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap()).unwrap();
                    if let Some(gate) = gate {
                        gate.acquire().await.unwrap().forget();
                    }
                    let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    // An explicitly cancelled utility may close its HTTP socket.
                    if socket.write_all(headers.as_bytes()).await.is_ok() {
                        let _ = socket.write_all(body.as_bytes()).await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
            while let Some(result) = workers.join_next().await {
                result.unwrap();
            }
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }
    pub async fn request(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
    pub fn no_request(&mut self) {
        assert!(matches!(
            self.requests.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub async fn host(http: &HttpFixture) -> Arc<Host> {
    let config = HostConfig {
        framework_instructions: "Transport framework".into(),
        models: vec![OpenAiModelSettings {
            model: "custom/provider/chat".parse().unwrap(),
            endpoint: http.endpoint.clone(),
            provider_model: "loopback-fixture".into(),
            api_key_env: "TASK040_MODEL_KEY".into(),
            context_window: 4096,
            supported_reasoning: vec![ReasoningPreference::Auto],
            supports_tools: true,
            request_timeout_ms: None,
        }],
        network: NetworkSettings {
            listen: None,
            access_key_env: Some("TASK040_HOST_KEY".into()),
        },
        ..HostConfig::default()
    };
    let credentials = HostCredentials {
        model_keys: vec![(
            config.models[0].model.clone(),
            SecretApiKey::new("task040-fixture-model-key").unwrap(),
        )]
        .into_iter()
        .collect(),
        network_access_key: Some(KEY.into()),
    };
    Host::memory_configured(
        HostResourceBuilder::builtins()
            .unwrap()
            .prepare_with_credentials(config, credentials)
            .await
            .unwrap(),
    )
    .unwrap()
}
pub async fn create(client: &HostClient) -> SessionClient {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "custom/provider/chat".parse().unwrap();
    config.session_defaults.execution.compaction = CompactionSettings {
        enabled: false,
        trigger_percent: 50,
        target_percent: 20,
    };
    let receipt = client
        .create_agent(
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "Compaction transport fixture".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
            options(),
        )
        .await
        .unwrap();
    client.session(SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    })
}
pub async fn seed(client: &HostClient, http: &mut HttpFixture) -> SessionClient {
    let session = create(client).await;
    for index in 0..2 {
        session
            .send(
                input(
                    &format!("task {index} {}", "recorded task ".repeat(100)),
                    MessageMode::FollowUp,
                ),
                options(),
            )
            .await
            .unwrap();
        let request = http.request().await;
        assert!(!is_utility(&request));
        settled(&session, index + 1).await;
    }
    session
}
pub fn is_utility(request: &Value) -> bool {
    request["messages"][0]["content"]
        .as_str()
        .is_some_and(|text| text.starts_with("Summarize historical conversation data"))
}
pub async fn settled(session: &SessionClient, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let turns = session.turns(options()).await.unwrap().data;
            if turns.len() == count
                && turns
                    .iter()
                    .all(|turn| turn.data.state.state == TurnState::Finished)
            {
                assert!(
                    turns
                        .iter()
                        .all(|turn| turn.data.state.outcome == Some(TurnOutcome::Completed)),
                    "{turns:?}"
                );
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
pub async fn finished(session: &SessionClient, operation: &OperationId) -> SessionSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = session.state(options()).await.unwrap();
            if let Some(activity) = &snapshot.data.compaction
                && matches!(&activity.source, CompactionSource::Manual { operation_id } if operation_id == operation)
                && activity.state.finished() {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap()
}
pub async fn history(session: &SessionClient) -> Value {
    serde_json::to_value(
        session
            .history(
                HistoryReadRequest {
                    cursor: None,
                    limit: 100,
                },
                options(),
            )
            .await
            .unwrap()
            .data,
    )
    .unwrap()
}
pub async fn retained(client: &HostClient, receipt: &CompactReceipt) {
    assert_eq!(
        client
            .receipt(receipt.operation_id.clone(), options())
            .await
            .unwrap(),
        Receipt::Compact(receipt.clone())
    );
}
pub fn subscription(target: &SessionRef) -> SubscribeRequest {
    SubscribeRequest {
        scope: SubscriptionScope::Session {
            agent_id: target.agent_id.clone(),
            session_id: target.session_id.clone(),
        },
        categories: [EventCategory::State].into_iter().collect(),
    }
}
pub async fn tls_server(host: Arc<Host>) -> super::wss_common::Server {
    let (server_tls, client_tls) = super::wss_common::certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(serve_wss(host, listener, server_tls));
    super::wss_common::Server {
        address,
        client_tls,
        task,
    }
}
pub struct Connected {
    pub client: HostClient,
    pipe: Option<JoinHandle<std::io::Result<()>>>,
    _tls: Option<super::wss_common::Server>,
}
impl Connected {
    pub async fn new(host: Arc<Host>, transport: u8) -> Self {
        match transport {
            0 => Self {
                client: HostClient::in_process(host).unwrap(),
                pipe: None,
                _tls: None,
            },
            1 => {
                let (peer, server) = tokio::io::duplex(1024);
                let (reader, writer) = tokio::io::split(peer);
                let (input, output) = tokio::io::split(server);
                let pipe = tokio::spawn(serve_stdio(host, input, output));
                Self {
                    client: HostClient::stdio(reader, writer, options()).await.unwrap(),
                    pipe: Some(pipe),
                    _tls: None,
                }
            }
            2 => {
                let tls = tls_server(host).await;
                Self {
                    client: tls.client(KEY).await.unwrap(),
                    pipe: None,
                    _tls: Some(tls),
                }
            }
            _ => unreachable!(),
        }
    }
    pub async fn finish(mut self) {
        self.client.close();
        if let Some(pipe) = self.pipe.take() {
            pipe.await.unwrap().unwrap();
        }
    }
}
impl Drop for Connected {
    fn drop(&mut self) {
        self.client.close();
        if let Some(pipe) = &self.pipe {
            pipe.abort();
        }
    }
}
