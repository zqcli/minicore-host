//! Production resource assembly with actual Host owners, OS pipes and loopback
//! TLS. The HTTP/SSE provider is a local fixture, never an external model.
mod os_pipe;
mod wss_common;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_transport::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
};

const MODEL_KEY: &str = "task027-fixture-only-model-secret";
const HOST_KEY: &str = "task027-fixture-only-host-secret";
const MODEL: &str = "provider-a/chat";

fn options() -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(5_000),
    }
}
fn value(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}
fn selection() -> Vec<ToolBinding> {
    vec![ToolBinding {
        name: "agent".into(),
    }]
}
fn agent_request(tools: Vec<ToolBinding>, session_tools: Vec<ToolBinding>) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = MODEL.parse().unwrap();
    config.session_defaults.tools = tools;
    config.session_defaults.execution.compaction.enabled = false;
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "Resource query acceptance".into(),
        mode: StorageMode::Memory,
        config,
        default_session: Some(SessionOptions {
            mode: None,
            overrides: Some(SessionOverrides(SessionConfigPatch {
                tools: Some(session_tools),
                ..Default::default()
            })),
        }),
        initial_message: None,
    }
}
fn target(receipt: &AgentCreateReceipt) -> SessionRef {
    SessionRef {
        agent_id: receipt.agent_id.clone().unwrap(),
        session_id: receipt.session_id.clone().unwrap(),
    }
}
fn agent_target(host: &HostClient, target: &SessionRef) -> AgentRef {
    AgentRef {
        host_id: host.initial_info().host_id.clone(),
        agent_id: target.agent_id.clone(),
    }
}
fn selected(target: ToolListTarget) -> ToolListRequest {
    ToolListRequest {
        target: Some(target),
    }
}
fn fault(error: CallError) -> Fault {
    match error {
        CallError::Business(fault) => fault,
        other => panic!("expected an owner Fault, got {other:?}"),
    }
}

struct HttpFixture {
    endpoint: String,
    requests: mpsc::UnboundedReceiver<Value>,
    task: JoinHandle<()>,
}
impl HttpFixture {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (send, requests) = mpsc::unbounded_channel();
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
                send.send(serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap())
                    .unwrap();
                let body = format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{"content":"Actual HTTP fixture response"},"finish_reason":null}]}),
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
                );
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

struct Resources {
    host: Arc<Host>,
    config: HostConfig,
    builder: HostResourceBuilder,
    provider: HttpFixture,
}
impl Resources {
    async fn new() -> Self {
        let provider = HttpFixture::start().await;
        let config = HostConfig {
            name: "Real configured Host".into(),
            framework_instructions: "Resource query acceptance framework".into(),
            // Deliberately reverse the two complete, same-leaf references.
            models: ["provider-b/chat", MODEL]
                .into_iter()
                .map(|name| OpenAiModelSettings {
                    api: minicore_host_core::OpenAiModelApi::ChatCompletions,
                    model: name.parse().unwrap(),
                    endpoint: provider.endpoint.clone(),
                    provider_model: "configured-chat".into(),
                    api_key_env: "TASK027_MODEL_KEY".into(),
                    context_window: 64_000,
                    supported_reasoning: vec![ReasoningPreference::Auto, ReasoningPreference::High],
                    supports_tools: true,
                    request_timeout_ms: None,
                })
                .collect(),
            tools: selection(),
            network: NetworkSettings {
                listen: None,
                access_key_env: Some("TASK027_HOST_KEY".into()),
            },
            logging: LogSettings::default(),
        };
        let builder = HostResourceBuilder::builtins().unwrap();
        let candidate = prepare(&builder, config.clone()).await;
        Self {
            host: Host::memory_configured(candidate).unwrap(),
            config,
            builder,
            provider,
        }
    }
}
async fn prepare(builder: &HostResourceBuilder, config: HostConfig) -> PreparedHostConfiguration {
    let credentials = HostCredentials {
        model_keys: config
            .models
            .iter()
            .map(|model| (model.model.clone(), SecretApiKey::new(MODEL_KEY).unwrap()))
            .collect(),
        network_access_key: Some(HOST_KEY.into()),
    };
    builder
        .prepare_with_credentials(config, credentials)
        .await
        .unwrap()
}
use wss_common::tls_server;
async fn pipe_client(host: Arc<Host>) -> (HostClient, JoinHandle<std::io::Result<()>>) {
    let (to_server, input) = crate::os_pipe::pipe().await.unwrap();
    let (output, from_server) = crate::os_pipe::pipe().await.unwrap();
    let task = tokio::spawn(serve_stdio(host, input, output));
    (
        HostClient::stdio(from_server, to_server, options())
            .await
            .unwrap(),
        task,
    )
}
use wss_common::Clients as QueryClients;
fn assert_public(value: &Value) {
    let serialized = serde_json::to_string(value).unwrap();
    assert!(!serialized.contains(MODEL_KEY));
    assert!(!serialized.contains(HOST_KEY));
    assert!(!serialized.contains("Bearer"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readonly_queries_share_real_config_models_and_explicit_tool_selections() {
    let mut resources = Resources::new().await;
    let operator = resources.host.connect().unwrap();
    let enabled = resources
        .host
        .create_agent_async(&operator.caller(), agent_request(vec![], selection()))
        .await
        .unwrap();
    let disabled = resources
        .host
        .create_agent_async(&operator.caller(), agent_request(selection(), vec![]))
        .await
        .unwrap();
    let expected_tools = resources.host.tool_list(&operator.caller()).unwrap();
    assert_eq!(expected_tools.len(), 1);
    assert_eq!(expected_tools[0].name().as_str(), "agent");
    let peers = QueryClients::new(resources.host.clone(), HOST_KEY, options()).await;
    for client in &peers.clients {
        assert!(!client.initial_info().can_operate);
        let config = client.config(options()).await.unwrap();
        assert_eq!(config.config, resources.config);
        assert_eq!(config.config_revision, 0);
        assert_public(&value(&config));
        let models = client.models(options()).await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].model.as_str(), MODEL);
        assert_eq!(models[1].model.as_str(), "provider-b/chat");
        assert_eq!(models[0].context_window, 64_000);
        assert_eq!(
            models[0].supported_reasoning,
            vec![ReasoningPreference::Auto, ReasoningPreference::High]
        );
        assert!(models[0].supports_tools);
        assert_public(&value(&models));
        assert!(value(&models)[0].get("kind").is_none());
        let available = client
            .tools(ToolListRequest::default(), options())
            .await
            .unwrap();
        assert_eq!(available, expected_tools);
        assert_public(&value(&available));
        for (receipt, agent_enabled, session_enabled) in
            [(&enabled, false, true), (&disabled, true, false)]
        {
            let session = target(receipt);
            let agent_tools = client
                .tools(
                    selected(ToolListTarget::Agent(agent_target(client, &session))),
                    options(),
                )
                .await
                .unwrap();
            let session_tools = client
                .tools(selected(ToolListTarget::Session(session)), options())
                .await
                .unwrap();
            assert_eq!(
                agent_tools,
                if agent_enabled {
                    expected_tools.clone()
                } else {
                    vec![]
                }
            );
            assert_eq!(
                session_tools,
                if session_enabled {
                    expected_tools.clone()
                } else {
                    vec![]
                }
            );
        }
        assert_eq!(
            fault(
                client
                    .create_agent(agent_request(vec![], vec![]), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostAccessReadOnly)
        );
    }
    // Pure resource queries and empty creation do not probe the provider.
    assert!(
        tokio::time::timeout(
            Duration::from_millis(25),
            resources.provider.requests.recv()
        )
        .await
        .is_err()
    );
    peers.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reload_queries_expose_real_removals_and_keep_missing_selections_as_faults() {
    let resources = Resources::new().await;
    let operator = resources.host.connect().unwrap();
    let created = resources
        .host
        .create_agent_async(&operator.caller(), agent_request(selection(), selection()))
        .await
        .unwrap();
    let session = target(&created);
    let peers = QueryClients::new(resources.host.clone(), HOST_KEY, options()).await;
    let mut next = resources.config.clone();
    next.name = "Reloaded resource set".into();
    next.models.clear();
    next.tools.clear();
    // This is the real prepared core API, not a claimed host.reload RPC route.
    assert_eq!(
        resources
            .host
            .reload_configuration(
                &operator.caller(),
                0,
                prepare(&resources.builder, next.clone()).await
            )
            .await
            .unwrap(),
        HostReloadResult::Applied { config_revision: 1 }
    );
    for client in &peers.clients {
        assert_eq!(
            client.config(options()).await.unwrap(),
            HostConfigSnapshot {
                config: next.clone(),
                config_revision: 1
            }
        );
        assert!(client.models(options()).await.unwrap().is_empty());
        assert!(
            client
                .tools(ToolListRequest::default(), options())
                .await
                .unwrap()
                .is_empty()
        );
        for selection in [
            ToolListTarget::Agent(agent_target(client, &session)),
            ToolListTarget::Session(session.clone()),
        ] {
            let missing = fault(
                client
                    .tools(selected(selection), options())
                    .await
                    .unwrap_err(),
            );
            assert_eq!(missing.kind(), Some(FaultKind::SessionResourceMissing));
            assert!(missing.message().contains("agent"));
        }
        // Removal must not rewrite the existing owner's selected configuration.
        let committed = client
            .session(session.clone())
            .info(options())
            .await
            .unwrap();
        assert_eq!(committed.config.model.as_str(), MODEL);
        assert_eq!(committed.config.tools, selection());
        let mut wrong_host = agent_target(client, &session);
        wrong_host.host_id = HostId::new().unwrap();
        assert_eq!(
            fault(
                client
                    .tools(selected(ToolListTarget::Agent(wrong_host)), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostOwnershipMismatch)
        );
        let missing_agent = AgentRef {
            host_id: client.initial_info().host_id.clone(),
            agent_id: AgentId::new().unwrap(),
        };
        assert_eq!(
            fault(
                client
                    .tools(selected(ToolListTarget::Agent(missing_agent)), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostAgentNotFound)
        );
        let missing_session = SessionRef {
            agent_id: session.agent_id.clone(),
            session_id: SessionId::new().unwrap(),
        };
        assert_eq!(
            fault(
                client
                    .tools(
                        selected(ToolListTarget::Session(missing_session)),
                        options()
                    )
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::AgentSessionNotFound)
        );
    }
    peers.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn known_resource_references_execute_real_http_without_prior_list_queries() {
    for transport in 0..3 {
        let mut resources = Resources::new().await;
        let mut pipe = None;
        let mut tls = None;
        let client = match transport {
            0 => HostClient::in_process(resources.host.clone()).unwrap(),
            1 => {
                let (client, server) = pipe_client(resources.host.clone()).await;
                pipe = Some(server);
                client
            }
            2 => {
                let server = tls_server(resources.host.clone()).await;
                let client = server.client(HOST_KEY).await.unwrap();
                tls = Some(server);
                client
            }
            _ => unreachable!(),
        };
        let receipt = client
            .create_agent(agent_request(selection(), selection()), options())
            .await
            .unwrap();
        let session = client.session(target(&receipt));
        session
            .send(
                MessageInput {
                    message_id: MessageId::new().unwrap(),
                    mode: MessageMode::FollowUp,
                    text: "Use the already known model and tool references".into(),
                    in_reply_to: None,
                },
                options(),
            )
            .await
            .unwrap();
        let request = resources.provider.request().await;
        assert_eq!(request["model"], "configured-chat");
        let turn = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(turn) = session.turns(options()).await.unwrap().data.first() {
                    break turn.turn_id.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let result = session.wait_turn(turn, options()).await.unwrap();
        assert_eq!(
            result.data.info.as_ref().unwrap().outcome,
            Some(TurnOutcome::Completed)
        );
        assert!(
            serde_json::to_string(&result.data.history)
                .unwrap()
                .contains("Actual HTTP fixture response")
        );
        // Query only after execution and compare to the actual provider payload.
        let specs = client
            .tools(
                selected(ToolListTarget::Session(target(&receipt))),
                options(),
            )
            .await
            .unwrap();
        assert_eq!(request["tools"].as_array().unwrap().len(), specs.len());
        assert_eq!(
            request["tools"][0]["function"]["name"],
            specs[0].name().as_str()
        );
        assert_eq!(
            request["tools"][0]["function"]["description"],
            specs[0].description().as_str()
        );
        assert_eq!(
            request["tools"][0]["function"]["parameters"],
            *specs[0].input_schema()
        );
        client.close();
        if let Some(server) = pipe {
            server.await.unwrap().unwrap();
        }
        drop(tls);
    }
}

struct RawPipe {
    reader: BufReader<crate::os_pipe::Receiver>,
    writer: crate::os_pipe::Sender,
    task: JoinHandle<std::io::Result<()>>,
}
impl RawPipe {
    async fn new(host: Arc<Host>) -> Self {
        let (writer, input) = crate::os_pipe::pipe().await.unwrap();
        let (output, reader) = crate::os_pipe::pipe().await.unwrap();
        Self {
            reader: BufReader::new(reader),
            writer,
            task: tokio::spawn(serve_stdio(host, input, output)),
        }
    }
    async fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        let request = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
        self.writer
            .write_all(serde_json::to_string(&request).unwrap().as_bytes())
            .await
            .unwrap();
        self.writer.write_all(b"\n").await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(5), self.reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    async fn finish(self) {
        drop(self.writer);
        self.task.await.unwrap().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resource_rpc_shape_and_validation_match_on_real_pipes_and_tls() {
    let resources = Resources::new().await;
    let _operator = resources.host.connect().unwrap();
    let mut pipe = RawPipe::new(resources.host.clone()).await;
    let tls = tls_server(resources.host.clone()).await;
    let mut wss = tls.raw().await;
    assert_eq!(
        pipe.request(0, "initialize", json!({})).await["result"]["can_operate"],
        false
    );
    assert_eq!(
        wss.initialize(HOST_KEY).await["result"]["can_operate"],
        false
    );
    for (index, (method, params)) in [
        ("host.config.get", json!({})),
        ("model.list", json!({})),
        ("tool.list", json!({})),
    ]
    .into_iter()
    .enumerate()
    {
        let stdio = pipe.request(index as i64 + 1, method, params.clone()).await;
        let network = wss.request(index as i64 + 1, method, params).await;
        assert!(stdio.get("error").is_none(), "{stdio}");
        assert_eq!(stdio, network);
        assert_public(&stdio);
    }
    let mixed = json!({"target":{"host_id":HostId::new().unwrap(),"agent_id":AgentId::new().unwrap(),"session_id":SessionId::new().unwrap()}});
    for (index, (method, params)) in [
        ("host.config.get", json!({"access_key":HOST_KEY})),
        ("model.list", json!({"target":MODEL})),
        ("tool.list", json!({"caller":{"can_operate":true}})),
        ("tool.list", json!({"target":{}})),
        ("tool.list", mixed),
    ]
    .into_iter()
    .enumerate()
    {
        let stdio = pipe
            .request(index as i64 + 10, method, params.clone())
            .await;
        let network = wss.request(index as i64 + 10, method, params).await;
        assert_eq!(stdio, network);
        assert_eq!(stdio["error"]["code"], -32602);
        assert_public(&stdio);
    }
    assert!(SUPPORTED_METHODS.contains(&"host.reload"));
    let invalid = pipe.request(30, "host.reload", json!({})).await;
    let network = wss.request(30, "host.reload", json!({})).await;
    assert_eq!(invalid, network);
    assert_eq!(invalid["error"]["code"], -32602);
    assert_eq!(
        pipe.request(31, "host.reload_config", json!({})).await["error"]["code"],
        -32601
    );
    pipe.finish().await;
}
