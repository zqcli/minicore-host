//! Explicit loopback model replies; all Host, Runtime, file and process work is real.
use crate::support::{self, Process, WAIT};
use minicore_host_core::{HostConfig, ObservationEvent, ObservationNotification};
use minicore_host_protocol::*;
use minicore_host_transport::{ClientSubscription, HostClient, tls};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task::JoinSet,
};

pub struct ModelRequest {
    pub body: Value,
    response: oneshot::Sender<String>,
}
impl ModelRequest {
    pub fn answer(self, answer: String) {
        self.response.send(answer).expect("live model request");
    }
    pub fn tool_result(&self, id: &str) -> Value {
        let matches: Vec<_> = self.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool" && message["tool_call_id"] == id)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "one actual native result for {id}: {}",
            self.body
        );
        serde_json::from_str(matches[0]["content"].as_str().unwrap()).unwrap()
    }
    pub fn declarations(&self, expected: &[&str]) {
        let mut names: Vec<_> = self.body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        let mut expected = expected.to_vec();
        names.sort();
        expected.sort();
        assert_eq!(names, expected);
    }
}

/// Each real HTTP request is a gate, including concurrent requests. The fixture
/// supplies only a model response, never a tool result or filesystem observation.
pub struct Provider {
    pub endpoint: String,
    requests: mpsc::UnboundedReceiver<ModelRequest>,
    task: tokio::task::JoinHandle<()>,
}
impl Provider {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (send, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut workers = JoinSet::new();
            loop {
                tokio::select! {
                    socket = listener.accept() => {
                        let (socket, _) = socket.unwrap();
                        let send = send.clone();
                        workers.spawn(serve(socket, send));
                    }
                    result = workers.join_next(), if !workers.is_empty() => {
                        result.unwrap().unwrap();
                    }
                }
            }
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }
    pub async fn request(&mut self) -> ModelRequest {
        tokio::time::timeout(WAIT, self.requests.recv())
            .await
            .unwrap()
            .expect("actual HTTP request")
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(mut socket: TcpStream, send: mpsc::UnboundedSender<ModelRequest>) {
    let mut bytes = Vec::new();
    let boundary = loop {
        if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break at + 4;
        }
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
    };
    let head = std::str::from_utf8(&bytes[..boundary]).unwrap();
    assert!(head.contains(&format!("Bearer {}", support::MODEL_KEY)));
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
    let (response, receive) = oneshot::channel();
    send.send(ModelRequest {
        body: serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap(),
        response,
    })
    .unwrap_or_else(|_| panic!("request consumer is live"));
    let Ok(body) = receive.await else { return };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(header.as_bytes()).await.unwrap();
    socket.write_all(body.as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
}
pub fn tool(id: &str, name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
    )
}
pub fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]})
    )
}
pub fn bindings(names: &[&str]) -> Vec<ToolBinding> {
    names
        .iter()
        .map(|name| ToolBinding {
            name: (*name).into(),
        })
        .collect()
}
pub fn agent(workspace: &Path, tools: &[&str]) -> Value {
    let mut request = support::agent_request();
    request["config"]["session_defaults"]["workspace"] = json!({"path":workspace});
    request["config"]["session_defaults"]["tools"] = json!(bindings(tools));
    request
}
pub struct App {
    pub directory: tempfile::TempDir,
    pub root: PathBuf,
    pub certificate: PathBuf,
    pub key: PathBuf,
    pub tls: Arc<tls::ClientConfig>,
    pub config: HostConfig,
    pub process: Process,
    pub observer: HostClient,
    pub initial: Value,
}
impl App {
    pub async fn start(provider: &Provider, tools: &[&str]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("data");
        let path = directory.path().join("config.json");
        let (certificate, key, tls) = support::certificates(directory.path());
        let mut config = support::config(Some(provider.endpoint.clone()), true);
        config.tools = bindings(tools);
        support::write_config(&path, &config);
        let mut process = Process::start(&root, Some(&path), true, Some((&certificate, &key)));
        let initial = process.rpc("initialize", json!({})).await;
        assert_eq!(initial["can_operate"], true);
        let observer = connect(&mut process, tls.clone()).await;
        assert!(!observer.initial_info().can_operate);
        Self {
            directory,
            root,
            certificate,
            key,
            tls,
            config,
            process,
            observer,
            initial,
        }
    }
    pub async fn create(&mut self, workspace: &Path, tools: &[&str]) -> Value {
        let created = self
            .process
            .rpc("agent.create", agent(workspace, tools))
            .await;
        support::target(&created)
    }
}
pub async fn connect(process: &mut Process, tls: Arc<tls::ClientConfig>) -> HostClient {
    let started = process.event("host.started").await;
    let port = started["attributes"]["listen_port"].as_u64().unwrap();
    HostClient::wss(
        &format!("wss://localhost:{port}/"),
        tls,
        support::HOST_KEY,
        support::options(),
    )
    .await
    .unwrap()
}
pub async fn subscribe(observer: &HostClient, target: &Value) -> ClientSubscription {
    observer
        .subscribe(
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: serde_json::from_value(target["agent_id"].clone()).unwrap(),
                    session_id: serde_json::from_value(target["session_id"].clone()).unwrap(),
                },
                categories: [
                    EventCategory::State,
                    EventCategory::Output,
                    EventCategory::Completion,
                ]
                .into_iter()
                .collect(),
            },
            support::options(),
        )
        .await
        .unwrap()
}
pub async fn event(events: &mut ClientSubscription) -> ObservationEvent {
    match tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap()
        .expect("live event stream")
    {
        ObservationNotification::Event(push) => push.event,
        other => panic!("unexpected resync or terminal event: {other:?}"),
    }
}
pub async fn only_turn(process: &mut Process, target: &Value) -> Value {
    let turns = process.rpc("turn.list", json!({"target":target})).await;
    assert_eq!(turns["data"].as_array().unwrap().len(), 1);
    turns["data"][0]["turn_id"].clone()
}
pub fn read(path: &str) -> Value {
    json!({"path":path,"start":0,"end":null,"max_bytes":4096})
}
pub fn list(kind: &str) -> Value {
    json!({"scope":{"kind":kind},"cursor":null,"limit":100})
}
pub fn diff(reference: &Value, comparison: &str) -> Value {
    json!({"change_ref":reference,"comparison":comparison,"context_lines":3,"max_bytes":4096,"cursor":null})
}
pub fn lines(result: &Value, kind: &str) -> String {
    result["data"]["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|hunk| hunk["lines"].as_array().unwrap())
        .filter(|line| line["kind"] == kind)
        .map(|line| line["text"].as_str().unwrap())
        .collect()
}
pub fn repository(path: &Path) {
    let result = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
pub async fn history(process: &mut Process, target: &Value) -> Value {
    let result = process
        .rpc(
            "session.history.read",
            json!({"target":target,"request":{"cursor":null,"limit":100}}),
        )
        .await;
    assert!(result["data"]["next_cursor"].is_null());
    result["data"]["items"].clone()
}
pub fn history_result<'a>(history: &'a Value, id: &str) -> &'a Value {
    let results: Vec<_> = history
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "tool_result" && item["tool_call_id"] == id)
        .collect();
    assert_eq!(results.len(), 1, "one persisted native result for {id}");
    &results[0]["result"]
}
pub fn history_path(root: &Path, target: &Value) -> PathBuf {
    root.join("agents")
        .join(target["agent_id"].as_str().unwrap())
        .join("sessions")
        .join(target["session_id"].as_str().unwrap())
        .join("history.jsonl")
}
pub fn fault(error: CallError) -> Fault {
    match error {
        CallError::Business(fault) => fault,
        other => panic!("expected business Fault, got {other:?}"),
    }
}
