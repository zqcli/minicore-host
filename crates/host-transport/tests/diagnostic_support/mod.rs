#![allow(dead_code)]
//! Production Host/resources/Runtime with a scripted loopback HTTP server.
//! No model double, fabricated owner events, or observation listener is used.
use minicore_host_core::{
    Host, HostConfig, MessageBinding, OpenAiModelSettings, TurnResultSnapshot,
};
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_transport::{HostClient, SessionClient, serve_stdio};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
    task::JoinHandle,
};

pub const MODEL: &str = "diagnostic/chat";
pub const KEY: &str = "PRIVATE_DIAGNOSTIC_MODEL_KEY";
pub const ROLE: &str = "PRIVATE_DIAGNOSTIC_ROLE";
pub const BODY: &str = "PRIVATE_DIAGNOSTIC_INPUT";
pub const ANSWER: &str = "PRIVATE_DIAGNOSTIC_ANSWER";
pub const FRAMEWORK: &str = "PRIVATE_DIAGNOSTIC_FRAMEWORK";
pub const ERROR_BODY: &str = "PRIVATE_DIAGNOSTIC_UPSTREAM_ERROR";
pub const PROVIDER_MODEL: &str = "PRIVATE_DIAGNOSTIC_PROVIDER_MODEL";
pub const WAIT: Duration = Duration::from_secs(5);

pub fn options() -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(5_000),
    }
}
pub fn input() -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: BODY.into(),
        in_reply_to: Some(MessageId::new().unwrap()),
    }
}
pub fn create(initial: Option<MessageInput>) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.role.instructions = ROLE.into();
    config.session_defaults.model = MODEL.parse().unwrap();
    config.session_defaults.execution.compaction.enabled = false;
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "Diagnostics acceptance".into(),
        mode: StorageMode::Memory,
        config,
        default_session: None,
        initial_message: initial,
    }
}
pub fn target(receipt: &AgentCreateReceipt) -> SessionRef {
    SessionRef {
        agent_id: receipt.agent_id.clone().unwrap(),
        session_id: receipt.session_id.clone().unwrap(),
    }
}
pub fn configuration(endpoint: &str) -> HostConfig {
    let mut config = HostConfig {
        framework_instructions: FRAMEWORK.into(),
        ..Default::default()
    };
    config.models.push(OpenAiModelSettings {
        model: MODEL.parse().unwrap(),
        endpoint: endpoint.into(),
        provider_model: PROVIDER_MODEL.into(),
        api_key_env: "TASK053_EXPLICIT_FIXTURE_ONLY".into(),
        context_window: 128_000,
        supported_reasoning: vec![ReasoningPreference::Auto],
        supports_tools: true,
        request_timeout_ms: Some(5_000),
    });
    config
}
pub async fn prepare(config: HostConfig) -> minicore_host_core::PreparedHostConfiguration {
    HostResourceBuilder::builtins()
        .unwrap()
        .prepare_with_credentials(
            config,
            HostCredentials {
                model_keys: [(MODEL.parse().unwrap(), SecretApiKey::new(KEY).unwrap())].into(),
                network_access_key: None,
            },
        )
        .await
        .unwrap()
}
pub async fn host(endpoint: &str) -> Arc<Host> {
    Host::memory_configured(prepare(configuration(endpoint)).await).unwrap()
}
pub async fn persistent_host(path: &std::path::Path, endpoint: &str) -> Arc<Host> {
    Host::open_configured(path, configuration(endpoint), |config| async {
        Ok(prepare(config).await)
    })
    .await
    .unwrap()
}
pub async fn pipes(host: Arc<Host>) -> (HostClient, JoinHandle<std::io::Result<()>>) {
    let (send, input) = crate::os_pipe::pipe().await.unwrap();
    let (output, receive) = crate::os_pipe::pipe().await.unwrap();
    let server = tokio::spawn(serve_stdio(host, input, output));
    (
        HostClient::stdio(receive, send, options()).await.unwrap(),
        server,
    )
}
pub async fn bound(session: &SessionClient, message: &MessageId) -> TurnId {
    let bound = tokio::time::timeout(WAIT, async {
        loop {
            let snapshot = session.message(message.clone(), options()).await.unwrap();
            if let MessageBinding::Bound { turn_id } = snapshot.data.binding {
                break turn_id;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await;
    match bound {
        Ok(turn) => turn,
        Err(_) => {
            let state = session.state(options()).await.unwrap();
            panic!(
                "message {message} did not bind; scheduling={:?}, block={:?}, preparing={}, queued={}",
                state.data.scheduling,
                state.data.block_reason.as_ref().map(|_| "present"),
                state.data.preparing,
                state.data.next_turn.len()
            );
        }
    }
}
pub async fn result(session: &SessionClient, message: &MessageId) -> TurnResultSnapshot {
    let turn = bound(session, message).await;
    session.wait_turn(turn, options()).await.unwrap()
}
pub async fn stop(host: &Arc<Host>) {
    let control = host.process_control();
    control
        .shutdown(HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        })
        .unwrap();
    tokio::time::timeout(WAIT, async {
        let mut current = control.snapshot().unwrap();
        while current.lifecycle != HostLifecycle::Stopped {
            current = control.changed(&current).await.unwrap();
        }
    })
    .await
    .unwrap();
}

pub struct Reply {
    pub status: u16,
    pub body: String,
    pub gate: Option<Arc<Semaphore>>,
}
impl Reply {
    pub fn success() -> Self {
        Self {
            status: 200,
            gate: None,
            body: format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":{"content":ANSWER},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
            ),
        }
    }
    pub fn auth_failure() -> Self {
        Self {
            status: 401,
            gate: None,
            body: json!({"error":{"code":"invalid_api_key","message":ERROR_BODY}}).to_string(),
        }
    }
    pub fn gated(gate: Arc<Semaphore>) -> Self {
        Self {
            gate: Some(gate),
            ..Self::success()
        }
    }
}
pub struct Http {
    pub endpoint: String,
    pub started: Arc<Semaphore>,
    calls: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}
impl Http {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let started = Arc::new(Semaphore::new(0));
        let accepted = started.clone();
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let boundary = loop {
                    if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break at + 4;
                    }
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(read, 0);
                    bytes.extend_from_slice(&chunk[..read]);
                };
                let head = std::str::from_utf8(&bytes[..boundary]).unwrap();
                assert!(head.contains(&format!("Bearer {KEY}")));
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
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(read, 0);
                    bytes.extend_from_slice(&chunk[..read]);
                }
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap());
                accepted.add_permits(1);
                if let Some(gate) = reply.gate {
                    gate.acquire().await.unwrap().forget();
                }
                let kind = if reply.status == 200 {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let head = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(reply.body.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        Self {
            endpoint,
            started,
            calls,
            task,
        }
    }
    pub fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    /// Stop only this scripted upstream fixture after the real client cancels.
    /// No successful response or graceful provider completion is claimed.
    pub async fn abort(self) {
        self.task.abort();
        assert!(self.task.await.is_err_and(|error| error.is_cancelled()));
    }
    pub async fn finish(self) -> Vec<Value> {
        tokio::time::timeout(WAIT, self.task)
            .await
            .unwrap()
            .unwrap();
        self.calls.lock().unwrap().clone()
    }
}
