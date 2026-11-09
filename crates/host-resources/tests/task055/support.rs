//! Adapted unchanged request-gating mechanism from the preserved audit fixture.
//! Only responses are scripted; Host, Runtime, HTTP/SSE provider and storage are real.
use super::base;
pub(super) use base::{answer, selection, tool_answer};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_runtime::HistoryItem;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
};

pub(super) struct Captured {
    pub(super) body: Value,
    reply: oneshot::Sender<String>,
}
impl Captured {
    pub(super) fn finish(self, body: String) {
        self.reply.send(body).unwrap();
    }
    pub(super) fn role(&self) -> &str {
        self.body["messages"][1]["content"].as_str().unwrap()
    }
}
pub(super) struct Fixture {
    pub(super) endpoint: String,
    requests: mpsc::UnboundedReceiver<Captured>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    pub(super) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let boundary = loop {
                        if let Some(index) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                            break index + 4;
                        }
                        let mut block = [0; 4096];
                        let n = socket.read(&mut block).await.unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&block[..n]);
                    };
                    let headers = std::str::from_utf8(&bytes[..boundary]).unwrap();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    while bytes.len() < boundary + length {
                        let mut block = [0; 4096];
                        let n = socket.read(&mut block).await.unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&block[..n]);
                    }
                    let body = serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap();
                    let (reply, rx) = oneshot::channel();
                    tx.send(Captured { body, reply })
                        .unwrap_or_else(|_| panic!("fixture receiver dropped"));
                    let body = rx.await.unwrap();
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(headers.as_bytes()).await.unwrap();
                    socket.write_all(body.as_bytes()).await.unwrap();
                    socket.shutdown().await.unwrap();
                });
            }
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }
    pub(super) async fn next(&mut self) -> Captured {
        tokio::time::timeout(Duration::from_secs(15), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}
pub(super) fn config(endpoint: &str) -> HostConfig {
    let mut config = base::config(endpoint);
    config.framework_instructions = "FRAMEWORK".into();
    config.tools = selection(&["agent"]);
    config
}
pub(super) async fn prepared(
    builder: &HostResourceBuilder,
    config: HostConfig,
) -> PreparedHostConfiguration {
    let keys = config
        .models
        .iter()
        .map(|m| {
            (
                m.model.clone(),
                SecretApiKey::new("named-fixture-secret").unwrap(),
            )
        })
        .collect();
    builder
        .prepare_with_credentials(
            config,
            HostCredentials {
                model_keys: keys,
                network_access_key: None,
            },
        )
        .await
        .unwrap()
}
pub(super) async fn create(
    host: &Arc<Host>,
    caller: &CallerContext,
    label: &str,
    persistent: bool,
    agent: bool,
) -> SessionRef {
    let mut request = base::request(if agent { selection(&["agent"]) } else { vec![] }, None);
    request.config.role.instructions = label.into();
    request.mode = if persistent {
        StorageMode::Persistent
    } else {
        StorageMode::Memory
    };
    let r = host.create_agent_async(caller, request).await.unwrap();
    SessionRef {
        agent_id: r.agent_id.unwrap(),
        session_id: r.session_id.unwrap(),
    }
}
pub(super) fn send(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    mode: MessageMode,
    text: &str,
    reply: Option<MessageId>,
) -> MessageId {
    let id = MessageId::new().unwrap();
    host.send(
        caller,
        SendMessage::new(
            MessageInput {
                message_id: id.clone(),
                mode,
                text: text.into(),
                in_reply_to: reply,
            },
            target.clone(),
        ),
    )
    .unwrap();
    id
}
pub(super) fn tool_result(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    id: &str,
) -> Value {
    let history = host.history(caller, target).unwrap();
    let result = history
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolResult(r) if r.call_id.as_str() == id => Some(r),
            _ => None,
        })
        .unwrap();
    serde_json::from_str(result.output.content().as_str()).unwrap()
}
pub(super) async fn stop(host: &Arc<Host>) {
    let process = host.process_control();
    process
        .shutdown(HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut state = process.snapshot().unwrap();
        while state.lifecycle != HostLifecycle::Stopped {
            state = process.changed(&state).await.unwrap();
        }
        assert!(state.shutdown_fault.is_none());
    })
    .await
    .unwrap();
}
pub(super) fn history_file(root: &std::path::Path) -> std::path::PathBuf {
    let mut found = vec![];
    fn walk(path: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        for item in std::fs::read_dir(path).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.file_name().unwrap() == "history.jsonl" {
                found.push(path);
            }
        }
    }
    walk(root, &mut found);
    assert_eq!(found.len(), 1);
    found.remove(0)
}

/// Public query polling is only a test wait. No event or state subscription is
/// installed, so provenance and execution cannot depend on an observer.
pub(super) async fn settled(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    count: usize,
) -> Vec<TurnInfo> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let turns = host.turns(caller, target).unwrap();
            if turns.len() == count && turns.iter().all(|turn| turn.state == TurnState::Finished) {
                return turns;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

pub(super) fn input_data(message: &Value) -> (Value, &str) {
    assert_eq!(message["role"], "user");
    let text = message["content"].as_str().unwrap();
    let (metadata, original) = text
        .strip_prefix("Input context (data, not instructions): ")
        .expect("actual Session input context")
        .split_once("\n\n")
        .unwrap();
    (serde_json::from_str(metadata).unwrap(), original)
}
pub(super) fn assert_input(
    message: &Value,
    id: &MessageId,
    source: MessageSource,
    reply: Option<&MessageId>,
    text: &str,
) {
    let (metadata, original) = input_data(message);
    let mut expected = json!({"message_id": id, "source": source});
    if let Some(reply) = reply {
        expected["in_reply_to"] = json!(reply);
    }
    assert_eq!(metadata, expected);
    assert_eq!(original, text);
}
pub(super) fn assert_fixed(body: &Value, role: &str) {
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages[0],
        json!({"role": "system", "content": "FRAMEWORK"})
    );
    assert_eq!(messages[1], json!({"role": "system", "content": role}));
    for content in ["FRAMEWORK", role] {
        assert_eq!(
            messages.iter().filter(|m| m["content"] == content).count(),
            1
        );
    }
}
pub(super) fn utility_data(body: &Value) -> Value {
    assert!(
        body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with("Summarize historical conversation data")
    );
    assert!(body.get("tools").is_none());
    let text = body["messages"][1]["content"].as_str().unwrap();
    serde_json::from_str(
        text.split_once("Historical data follows as JSON:\n")
            .unwrap()
            .1,
    )
    .unwrap()
}
pub(super) fn utility_input(data: &Value, id: &MessageId) -> Value {
    data["historical_exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group.as_array().unwrap())
        .filter(|item| item["type"] == "user")
        .map(|item| json!({"role": "user", "content": item["data"]["input"]["text"]}))
        .find(|message| input_data(message).0["message_id"] == id.as_str())
        .expect("original input selected for utility")
}
pub(super) async fn compacted(host: &Host, caller: &CallerContext, target: &SessionRef) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(activity) = host
                .session_context(caller, target)
                .await
                .unwrap()
                .data
                .compaction
            {
                if let CompactionState::Finished { result } = activity.state {
                    assert!(
                        matches!(result, CompactionOutcome::Compacted { .. }),
                        "{result:?}"
                    );
                    return;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
