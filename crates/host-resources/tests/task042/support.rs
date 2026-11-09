//! Explicit HTTP/SSE fixture. Production Host, provider, Runtime and tools are real.
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, mpsc},
};

pub(crate) struct Server {
    pub(crate) endpoint: String,
    requests: mpsc::UnboundedReceiver<Value>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    pub(crate) async fn start(replies: Vec<(String, Option<Arc<Semaphore>>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (send, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for (body, gate) in replies {
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
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(body.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }
    pub(crate) async fn request(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(10), self.requests.recv())
            .await
            .expect("HTTP fixture request")
            .expect("HTTP fixture still running")
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub(crate) fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
    )
}
pub(crate) fn tool_answer(id: &str, name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
    )
}
pub(crate) fn selection(names: &[&str]) -> Vec<ToolBinding> {
    names
        .iter()
        .map(|name| ToolBinding {
            name: (*name).into(),
        })
        .collect()
}
pub(crate) fn config(endpoint: &str) -> HostConfig {
    HostConfig {
        name: "Production local tools fixture".into(),
        framework_instructions: "FRAMEWORK A".into(),
        models: vec![OpenAiModelSettings {
            model: "custom/provider/chat".parse().unwrap(),
            endpoint: endpoint.into(),
            provider_model: "provider-A".into(),
            api_key_env: "TASK042_MODEL_KEY".into(),
            context_window: 128_000,
            supported_reasoning: vec![ReasoningPreference::Auto],
            supports_tools: true,
            request_timeout_ms: None,
        }],
        tools: selection(&["read", "write", "edit", "bash"]),
        network: NetworkSettings {
            listen: None,
            access_key_env: None,
        },
        logging: LogSettings::default(),
    }
}
pub(crate) fn keys(config: &HostConfig) -> HostCredentials {
    HostCredentials {
        model_keys: config
            .models
            .iter()
            .map(|model| {
                (
                    model.model.clone(),
                    SecretApiKey::new("explicit-fixture-secret").unwrap(),
                )
            })
            .collect(),
        network_access_key: None,
    }
}
pub(crate) async fn prepare(
    builder: &HostResourceBuilder,
    config: HostConfig,
) -> PreparedHostConfiguration {
    let credentials = keys(&config);
    builder
        .prepare_with_credentials(config, credentials)
        .await
        .unwrap()
}
pub(crate) fn request(tools: Vec<ToolBinding>, workspace: Option<WorkspaceRef>) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "custom/provider/chat".parse().unwrap();
    config.session_defaults.tools = tools;
    config.session_defaults.workspace = workspace;
    config.session_defaults.execution.compaction.enabled = false;
    // These resource/ordinary-tool fixtures exercise execution, not approval.
    config.session_defaults.execution.approval = ApprovalMode::Auto;
    config.role.instructions = "ROLE A".into();
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "Production tools".into(),
        mode: StorageMode::Memory,
        config,
        default_session: None,
        initial_message: None,
    }
}
pub(crate) fn create(
    host: &Host,
    caller: &CallerContext,
    tools: Vec<ToolBinding>,
    workspace: Option<WorkspaceRef>,
) -> SessionRef {
    let receipt = host
        .create_agent(caller, request(tools, workspace))
        .unwrap();
    SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    }
}
pub(crate) fn workspace(path: &std::path::Path) -> Option<WorkspaceRef> {
    Some(WorkspaceRef {
        path: path.to_str().unwrap().into(),
    })
}
pub(crate) fn send(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    text: &str,
) -> MessageId {
    let message_id = MessageId::new().unwrap();
    host.send(
        caller,
        SendMessage::new(
            MessageInput {
                message_id: message_id.clone(),
                mode: MessageMode::FollowUp,
                text: text.into(),
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
    message_id
}
/// Wait on owner state notifications, never infer completion from an elapsed delay.
pub(crate) async fn settled(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    count: usize,
) -> Vec<TurnInfo> {
    let mut states = host.session_states(caller, target).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let turns = host.turns(caller, target).unwrap();
            if turns.len() == count && turns.iter().all(|turn| turn.state == TurnState::Finished) {
                return turns;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .expect("actual Turn completion")
}
