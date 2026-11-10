#![allow(dead_code)]
use minicore_host_core::{HostConfig, OpenAiModelSettings};
use minicore_host_protocol::*;
use minicore_host_transport::tls;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

pub const MODEL_KEY: &str = "app-fixture-only-model-credential";
pub const HOST_KEY: &str = "app-fixture-only-network-credential";
pub const PRIVATE_PROMPT: &str = "private fixture framework prompt body";
pub const MODEL: &str = "fixture/chat";
pub const WAIT: Duration = Duration::from_secs(8);

pub fn config(endpoint: Option<String>, network: bool) -> HostConfig {
    let mut config = HostConfig {
        framework_instructions: PRIVATE_PROMPT.into(),
        ..Default::default()
    };
    if let Some(endpoint) = endpoint {
        config.models.push(OpenAiModelSettings {
            model: MODEL.parse().unwrap(),
            endpoint,
            provider_model: "local-http-fixture".into(),
            api_key_env: "MINICORE_APP_MODEL_KEY".into(),
            context_window: 128_000,
            supported_reasoning: vec![ReasoningPreference::Auto],
            supports_tools: true,
            request_timeout_ms: Some(20_000),
        });
    }
    if network {
        config.network.listen = Some("127.0.0.1:0".parse().unwrap());
        config.network.access_key_env = Some("MINICORE_APP_HOST_KEY".into());
    }
    config
}
pub fn write_config(path: &Path, config: &HostConfig) {
    std::fs::write(path, serde_json::to_vec(config).unwrap()).unwrap();
}
pub fn agent_request() -> Value {
    let mut config = AgentConfig::default();
    config.session_defaults.model = MODEL.parse().unwrap();
    config.session_defaults.execution.compaction.enabled = false;
    json!({ "operation_id": OperationId::new().unwrap(), "name": "Application fixture", "mode": "persistent", "config": config })
}
pub fn shutdown(strategy: &str) -> Value {
    json!({ "operation_id": OperationId::new().unwrap(), "strategy": strategy })
}
pub fn message(target: &Value, text: &str) -> Value {
    json!({ "target": { "kind":"session", "agent_id":target["agent_id"], "session_id":target["session_id"] }, "message_id": MessageId::new().unwrap(), "mode":"follow_up", "text":text })
}
pub fn target(receipt: &Value) -> Value {
    json!({"agent_id": receipt["agent_id"], "session_id": receipt["session_id"]})
}

#[derive(Default)]
struct Streams {
    stdout: Vec<String>,
    stderr: Vec<String>,
}
pub struct Process {
    child: Child,
    input: Option<ChildStdin>,
    stdout: mpsc::UnboundedReceiver<String>,
    stderr: mpsc::UnboundedReceiver<String>,
    streams: Arc<Mutex<Streams>>,
    readers: Vec<JoinHandle<()>>,
    next_id: u64,
}
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
}
impl Captured {
    pub fn protocol_only(&self) {
        for line in &self.stdout {
            assert_eq!(
                serde_json::from_str::<Value>(line).unwrap()["jsonrpc"],
                "2.0"
            );
        }
    }
    pub fn safe_diagnostics(&self) {
        for line in &self.stderr {
            for secret in [MODEL_KEY, HOST_KEY, PRIVATE_PROMPT] {
                assert!(!line.contains(secret), "diagnostic exposed fixture secret");
            }
        }
    }
    pub fn events(&self) -> Vec<Value> {
        self.stderr
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}
impl Process {
    pub fn start(
        root: &Path,
        config: Option<&Path>,
        stdio: bool,
        tls: Option<(&Path, &Path)>,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minicore-host"));
        command.arg("--data-root").arg(root);
        if let Some(config) = config {
            command.arg("--config").arg(config);
        }
        if stdio {
            command.arg("--stdio");
        }
        if let Some((cert, key)) = tls {
            command
                .arg("--tls-cert")
                .arg(cert)
                .arg("--tls-key")
                .arg(key);
        }
        command
            .env("MINICORE_APP_MODEL_KEY", MODEL_KEY)
            .env("MINICORE_APP_HOST_KEY", HOST_KEY);
        Self::command(command)
    }
    pub fn command(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let streams = Arc::new(Mutex::new(Streams::default()));
        let (out, stdout) = mpsc::unbounded_channel();
        let (err, stderr) = mpsc::unbounded_channel();
        let captured = streams.clone();
        let reader = child.stdout.take().unwrap();
        let out_thread = std::thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                let line = line.unwrap();
                captured.lock().unwrap().stdout.push(line.clone());
                let _ = out.send(line);
            }
        });
        let captured = streams.clone();
        let reader = child.stderr.take().unwrap();
        let err_thread = std::thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                let line = line.unwrap();
                captured.lock().unwrap().stderr.push(line.clone());
                let _ = err.send(line);
            }
        });
        Self {
            child,
            input,
            stdout,
            stderr,
            streams,
            readers: vec![out_thread, err_thread],
            next_id: 0,
        }
    }
    pub fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let line = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
        writeln!(self.input.as_mut().unwrap(), "{line}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        id
    }
    pub async fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        tokio::time::timeout(WAIT, async {
            loop {
                let line = self.stdout.recv().await.unwrap_or_else(|| {
                    panic!(
                        "process closed stdout: {:?}",
                        self.streams.lock().unwrap().stderr
                    )
                });
                let reply: Value = serde_json::from_str(&line).unwrap();
                if reply["id"] == id {
                    assert!(reply.get("error").is_none(), "{reply}");
                    return reply["result"].clone();
                }
            }
        })
        .await
        .unwrap()
    }
    pub async fn event(&mut self, component: &str, name: &str) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let line =
                    self.stderr.recv().await.unwrap_or_else(|| {
                        panic!("process closed stderr before {component}/{name}")
                    });
                if let Ok(record) = serde_json::from_str::<Value>(&line)
                    && record["component"] == component
                    && record["event"] == name
                {
                    return record;
                }
            }
        })
        .await
        .unwrap()
    }
    pub fn raw_input(&mut self, bytes: &[u8]) {
        self.input.as_mut().unwrap().write_all(bytes).unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }
    pub fn close_input(&mut self) {
        self.input.take();
    }
    pub fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
    #[cfg(unix)]
    pub fn signal(&self, signal: &str) {
        assert!(
            Command::new("kill")
                .args([signal, &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
    pub async fn finish(mut self) -> Captured {
        let status = tokio::time::timeout(WAIT, async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "process did not exit; stderr={:?}",
                self.streams.lock().unwrap().stderr
            )
        });
        self.input.take();
        for reader in self.readers.drain(..) {
            reader.join().unwrap();
        }
        let mut streams = self.streams.lock().unwrap();
        Captured {
            status,
            stdout: std::mem::take(&mut streams.stdout),
            stderr: std::mem::take(&mut streams.stderr),
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct HttpFixture {
    pub endpoint: String,
    entered: mpsc::UnboundedReceiver<Value>,
    release: mpsc::UnboundedSender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl HttpFixture {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (send, entered) = mpsc::unbounded_channel();
        let (release, mut released) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
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
                assert!(head.contains(&format!("Bearer {MODEL_KEY}")));
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
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
                let partial = format!(
                    "data: {}\n\n",
                    json!({"choices":[{"index":0,"delta":{"content":"actual local HTTP response"},"finish_reason":null}]})
                );
                socket
                    .write_all(format!("{:x}\r\n{}\r\n", partial.len(), partial).as_bytes())
                    .await
                    .unwrap();
                socket.flush().await.unwrap();
                send.send(serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap())
                    .unwrap();
                if released.recv().await.is_none() {
                    return;
                }
                let tail = format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
                );
                if socket
                    .write_all(format!("{:x}\r\n{}\r\n0\r\n\r\n", tail.len(), tail).as_bytes())
                    .await
                    .is_err()
                {
                    continue;
                }
                let _ = socket.shutdown().await;
            }
        });
        Self {
            endpoint,
            entered,
            release,
            task,
        }
    }
    pub async fn request(&mut self) -> Value {
        tokio::time::timeout(WAIT, self.entered.recv())
            .await
            .unwrap()
            .unwrap()
    }
    pub fn finish_response(&self) {
        self.release.send(()).unwrap();
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn certificates(
    dir: &Path,
) -> (
    std::path::PathBuf,
    std::path::PathBuf,
    Arc<tls::ClientConfig>,
) {
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = generated.cert.der().clone();
    let cert_path = dir.join("tls-cert.pem");
    let key_path = dir.join("tls-key.pem");
    std::fs::write(&cert_path, pem("CERTIFICATE", &cert)).unwrap();
    std::fs::write(
        &key_path,
        pem("PRIVATE KEY", &generated.signing_key.serialize_der()),
    )
    .unwrap();
    let mut roots = tls::RootCertStore::empty();
    roots.add(cert).unwrap();
    let client = tls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (cert_path, key_path, Arc::new(client))
}
fn pem(label: &str, bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            text.push(if index > chunk.len() {
                '='
            } else {
                ALPHABET[((bits >> (18 - index * 6)) & 63) as usize] as char
            });
        }
    }
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in text.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}
pub fn options() -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(5_000),
    }
}
