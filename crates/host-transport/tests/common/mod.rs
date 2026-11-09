#![allow(dead_code)]
//! Explicit model/resource fixtures. Host owners and Runtime remain real.
use futures_util::{StreamExt, stream};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf},
    sync::Semaphore,
    task::JoinHandle,
};

pub struct FixtureModel {
    descriptor: ModelDescriptor,
    pub started: Semaphore,
    pub finish: Arc<Semaphore>,
    pub calls: AtomicUsize,
}
impl Model for FixtureModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(
        &'a self,
        _request: ModelRequest,
        _context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.add_permits(1);
        let finish = Arc::clone(&self.finish);
        Box::pin(async move {
            let first = stream::iter([Ok(ModelEvent::text_delta("fixture output"))]);
            let last = stream::once(async move {
                finish.acquire_owned().await.unwrap().forget();
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                })
            });
            Ok(Box::pin(first.chain(last)) as ModelStream)
        })
    }
}
pub struct ValidationGate {
    pub entered: Semaphore,
    released: Mutex<bool>,
    ready: Condvar,
}
impl ValidationGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            released: Mutex::new(false),
            ready: Condvar::new(),
        })
    }
    pub fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.ready.notify_all();
    }
    fn block(&self) {
        self.entered.add_permits(1);
        let released = self.released.lock().unwrap();
        let (released, timed) = self
            .ready
            .wait_timeout_while(released, Duration::from_secs(10), |done| !*done)
            .unwrap();
        assert!(
            *released && !timed.timed_out(),
            "test did not release its validation gate"
        );
    }
}
pub struct FixtureResources {
    pub model: Arc<FixtureModel>,
    pub resolve_failure: AtomicBool,
    pub validate_calls: AtomicUsize,
    pub gate: Mutex<Option<(usize, Arc<ValidationGate>)>>,
}
impl ResourceResolver for FixtureResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        let number = self.validate_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let gate = self.gate.lock().unwrap().clone();
        if let Some((at, gate)) = gate
            && at == number
        {
            gate.block();
        }
        if config.model.as_str() == "fixture/model" {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "fixture model is missing",
            ))
        }
    }
    fn resolve<'a>(&'a self, _config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            if self.resolve_failure.load(Ordering::SeqCst) {
                return Err(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "fixture resource resolution failed",
                ));
            }
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
pub fn fixture() -> (Arc<Host>, Arc<FixtureResources>) {
    let model = Arc::new(FixtureModel {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128_000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        started: Semaphore::new(0),
        finish: Arc::new(Semaphore::new(0)),
        calls: AtomicUsize::new(0),
    });
    let resources = Arc::new(FixtureResources {
        model,
        resolve_failure: AtomicBool::new(false),
        validate_calls: AtomicUsize::new(0),
        gate: Mutex::new(None),
    });
    (Host::memory(resources.clone()).unwrap(), resources)
}
pub fn create_request() -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "transport fixture".into(),
        mode: StorageMode::Memory,
        config,
        default_session: None,
        initial_message: None,
    }
}
pub fn input(text: &str) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: text.into(),
        in_reply_to: None,
    }
}
pub fn options(ms: u64) -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(ms),
    }
}
pub fn control() -> SessionControlRequest {
    SessionControlRequest {
        operation_id: OperationId::new().unwrap(),
    }
}
pub async fn create(client: &HostClient) -> SessionClient {
    let receipt = client
        .create_agent(create_request(), options(2_000))
        .await
        .unwrap();
    client.session(SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    })
}
pub async fn stdio(host: Arc<Host>) -> (HostClient, JoinHandle<std::io::Result<()>>) {
    let (client, server) = tokio::io::duplex(256);
    let (reader, writer) = tokio::io::split(client);
    let (input, output) = tokio::io::split(server);
    let server = tokio::spawn(serve_stdio(host, input, output));
    (
        HostClient::stdio(reader, writer, options(2_000))
            .await
            .unwrap(),
        server,
    )
}
#[cfg(unix)]
pub async fn stdio_pipes(host: Arc<Host>) -> (HostClient, JoinHandle<std::io::Result<()>>) {
    let (to_server, server_input) = tokio::net::unix::pipe::pipe().unwrap();
    let (server_output, from_server) = tokio::net::unix::pipe::pipe().unwrap();
    let server = tokio::spawn(serve_stdio(host, server_input, server_output));
    (
        HostClient::stdio(from_server, to_server, options(2_000))
            .await
            .unwrap(),
        server,
    )
}
pub struct RawPeer {
    pub reader: BufReader<ReadHalf<DuplexStream>>,
    pub writer: WriteHalf<DuplexStream>,
}
impl RawPeer {
    pub async fn write(&mut self, value: Value) {
        self.bytes(&serde_json::to_vec(&value).unwrap()).await;
    }
    pub async fn bytes(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).await.unwrap();
        self.writer.write_all(b"\n").await.unwrap();
    }
    pub async fn read(&mut self) -> Value {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(3), self.reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    pub async fn request(&mut self, id: Value, method: &str, params: Value) -> Value {
        self.write(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .await;
        self.read().await
    }
    pub async fn initialize(&mut self) -> Value {
        self.request(json!("init"), "initialize", json!({})).await
    }
}
pub fn raw(host: Arc<Host>) -> (RawPeer, JoinHandle<std::io::Result<()>>) {
    let (client, server) = tokio::io::duplex(256);
    let (reader, writer) = tokio::io::split(client);
    let (input, output) = tokio::io::split(server);
    (
        RawPeer {
            reader: BufReader::new(reader),
            writer,
        },
        tokio::spawn(serve_stdio(host, input, output)),
    )
}
pub async fn running(session: &SessionClient) -> TurnId {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(id) = session
                .state(options(2_000))
                .await
                .unwrap()
                .data
                .active_turn_id
            {
                return id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
pub fn value<T: serde::Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap()
}
