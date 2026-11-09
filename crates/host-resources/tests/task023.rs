//! Real Host + Runtime + production HTTP/SSE on loopback, never a live provider.
#[path = "../../host-core/tests/support/input_context.rs"]
mod input_context;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_tools::{ToolFactory, ToolRegistry, TurnPort};
use minicore_runtime::{
    HistoryItem,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSpec,
    },
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Semaphore, mpsc},
};

struct Server {
    endpoint: String,
    requests: mpsc::UnboundedReceiver<Value>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start(replies: Vec<(String, Option<Arc<Semaphore>>)>) -> Self {
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
    async fn request(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
    )
}
fn tool_answer() -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"probe-call","type":"function","function":{"name":"probe","arguments":"{}"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
    )
}
fn config(endpoint: &str) -> HostConfig {
    HostConfig {
        name: "Configured Host".into(),
        framework_instructions: "FRAMEWORK A".into(),
        models: vec![OpenAiModelSettings {
            model: "custom/provider/chat".parse().unwrap(),
            endpoint: endpoint.into(),
            provider_model: "provider-A".into(),
            api_key_env: "TASK023_MODEL_KEY".into(),
            context_window: 128_000,
            supported_reasoning: vec![ReasoningPreference::Auto],
            supports_tools: true,
            request_timeout_ms: None,
        }],
        tools: vec![],
        network: NetworkSettings {
            listen: None,
            access_key_env: Some("TASK023_HOST_KEY".into()),
        },
        logging: LogSettings::default(),
    }
}
fn keys(config: &HostConfig, key: &str) -> HostCredentials {
    HostCredentials {
        model_keys: config
            .models
            .iter()
            .map(|m| {
                (
                    m.model.clone(),
                    SecretApiKey::new("fixture-model-secret").unwrap(),
                )
            })
            .collect(),
        network_access_key: config.network.access_key_env.as_ref().map(|_| key.into()),
    }
}
async fn prepare(
    builder: &HostResourceBuilder,
    config: HostConfig,
    key: &str,
) -> PreparedHostConfiguration {
    let credentials = keys(&config, key);
    builder
        .prepare_with_credentials(config, credentials)
        .await
        .unwrap()
}
fn request(tools: Vec<ToolBinding>, workspace: Option<WorkspaceRef>) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "custom/provider/chat".parse().unwrap();
    config.session_defaults.tools = tools;
    config.session_defaults.workspace = workspace;
    config.session_defaults.execution.compaction.enabled = false;
    config.role.instructions = "ROLE A".into();
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "Resource test".into(),
        mode: StorageMode::Memory,
        config,
        default_session: None,
        initial_message: None,
    }
}
fn create(
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
fn send(host: &Host, caller: &CallerContext, target: &SessionRef, text: &str) -> MessageId {
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
async fn settled(host: &Host, caller: &CallerContext, target: &SessionRef, count: usize) {
    let mut states = host.session_states(caller, target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let turns = host.turns(caller, target).unwrap();
            if turns.len() == count && turns.iter().all(|turn| turn.state == TurnState::Finished) {
                assert!(
                    turns
                        .iter()
                        .all(|turn| turn.outcome == Some(TurnOutcome::Completed)),
                    "{turns:?}"
                );
                return;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
async fn blocked(host: &Host, caller: &CallerContext, target: &SessionRef) -> SessionSnapshot {
    let mut states = host.session_states(caller, target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = states.snapshot().unwrap();
            if state.data.block_reason.is_some() && !state.data.preparing {
                return state;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}
struct Probe {
    spec: ToolSpec,
    calls: Arc<AtomicUsize>,
}
impl ToolFactory for Probe {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn bind(&self, _: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault> {
        Ok(Arc::new(Self {
            spec: self.spec.clone(),
            calls: self.calls.clone(),
        }))
    }
}
impl Tool for Probe {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("ACTUAL PROBE RESULT").unwrap(),
            ))
        })
    }
}
fn builder_with_probe() -> (HostResourceBuilder, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    for name in ["probe", "unselected"] {
        tools
            .register(Arc::new(Probe {
                spec: ToolSpec::new(
                    name.parse().unwrap(),
                    format!("{name} fixture"),
                    json!({"type":"object","properties":{}}),
                )
                .unwrap(),
                calls: calls.clone(),
            }))
            .unwrap();
    }
    (HostResourceBuilder::new(Arc::new(tools)), calls)
}
fn selection() -> Vec<ToolBinding> {
    vec![ToolBinding {
        name: "probe".into(),
    }]
}

/// Explicit fixture credentials, while all resource preparation still uses
/// the real production assembler and its original static ToolRegistry.
struct ExplicitCredentialsPreparer {
    builder: Arc<HostResourceBuilder>,
}
impl HostConfigurationPreparer for ExplicitCredentialsPreparer {
    fn prepare(&self, config: HostConfig) -> HostConfigurationFuture<'_> {
        Box::pin(async move {
            let credentials = keys(&config, "rotated-key");
            self.builder
                .prepare_with_credentials(config, credentials)
                .await
        })
    }
}
async fn applied(host: &Host, caller: &CallerContext, operation: &OperationId) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let receipt = host.receipt_get(caller, operation).unwrap();
            match receipt {
                Receipt::HostReload(HostReloadReceipt {
                    result: HostReloadResult::Applied { .. },
                    ..
                }) => return,
                Receipt::HostReload(HostReloadReceipt {
                    result: HostReloadResult::Preparing { .. } | HostReloadResult::Applying { .. },
                    ..
                }) => {}
                other => panic!("reload did not apply: {other:?}"),
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn old_turn_keeps_actual_model_tools_role_and_workspace_after_reload() {
    old_turn_resources_after_reload(false).await;
}

#[tokio::test]
async fn canonical_reload_receipt_keeps_real_old_turn_and_new_turn_resource_bindings() {
    old_turn_resources_after_reload(true).await;
}

async fn old_turn_resources_after_reload(canonical: bool) {
    let gate = Arc::new(Semaphore::new(0));
    let mut server_a = Server::start(vec![
        (tool_answer(), Some(gate.clone())),
        (answer("A finished"), None),
    ])
    .await;
    let mut server_b = Server::start(vec![(answer("B finished"), None)]).await;
    let directory = tempfile::tempdir().unwrap();
    let (builder, calls) = builder_with_probe();
    let builder = Arc::new(builder);
    let mut original = config(&server_a.endpoint);
    original.tools = selection();
    let host = Host::memory_configured(
        prepare(&builder, original.clone(), "old-key")
            .await
            .with_preparer(Arc::new(ExplicitCredentialsPreparer {
                builder: builder.clone(),
            })),
    )
    .unwrap();
    let mut connection = if canonical {
        host.connect_network("old-key").unwrap()
    } else {
        host.connect().unwrap()
    };
    let mut caller = connection.caller();
    let local_reader = host.connect().unwrap();
    let old_network = host.connect_network("old-key").unwrap();
    let target = create(
        &host,
        &caller,
        selection(),
        Some(WorkspaceRef {
            path: directory.path().to_str().unwrap().into(),
        }),
    );
    send(&host, &caller, &target, "first task");
    let first = server_a.request().await;
    let mut candidate = original.clone();
    candidate.framework_instructions = "FRAMEWORK B".into();
    candidate.models[0].endpoint = server_b.endpoint.clone();
    candidate.models[0].provider_model = "provider-B".into();
    candidate.tools.clear();
    if canonical {
        let operation_id = OperationId::new().unwrap();
        let formed = host
            .reload(
                &caller,
                HostReloadRequest {
                    operation_id: operation_id.clone(),
                    expected_revision: 0,
                    candidate,
                },
            )
            .unwrap();
        assert_eq!(
            formed.result,
            HostReloadResult::Preparing { config_revision: 0 }
        );
        applied(&host, &local_reader.caller(), &operation_id).await;
        assert!(host.info(&caller).is_err());
        connection = host.connect_network("rotated-key").unwrap();
        assert!(connection.initial_info().can_operate);
        caller = connection.caller();
        assert!(matches!(
            host.receipt_get(&caller, &operation_id).unwrap(),
            Receipt::HostReload(HostReloadReceipt {
                result: HostReloadResult::Applied { config_revision: 1 },
                ..
            })
        ));
        // This already-returned control fact was not rewritten or resent.
        assert_eq!(
            formed.result,
            HostReloadResult::Preparing { config_revision: 0 }
        );
    } else {
        assert_eq!(
            host.reload_configuration(
                &caller,
                0,
                prepare(&builder, candidate, "rotated-key").await,
            )
            .await
            .unwrap(),
            HostReloadResult::Applied { config_revision: 1 }
        );
    }
    assert!(host.info(&old_network.caller()).is_err());
    assert!(host.info(&local_reader.caller()).is_ok());
    assert!(host.info(&caller).is_ok());
    host.update_session_async(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            role: Some(Role {
                name: "Changed".into(),
                description: String::new(),
                instructions: "ROLE B".into(),
            }),
            tools: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    gate.add_permits(1);
    let second = server_a.request().await;
    settled(&host, &caller, &target, 1).await;
    assert_eq!(calls.load(Ordering::Acquire), 1);
    for body in [&first, &second] {
        assert_eq!(body["model"], "provider-A");
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["function"]["name"], "probe");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["content"] == "FRAMEWORK A")
                .count(),
            1
        );
        assert_eq!(
            messages.iter().filter(|m| m["content"] == "ROLE A").count(),
            1
        );
        assert!(messages.iter().any(|m| {
            m["content"]
                .as_str()
                .is_some_and(|s| s.contains(directory.path().to_str().unwrap()))
        }));
    }
    assert!(
        second["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool" && m["content"] == "ACTUAL PROBE RESULT")
    );
    send(&host, &caller, &target, "next task");
    let third = server_b.request().await;
    settled(&host, &caller, &target, 2).await;
    assert_eq!(third["model"], "provider-B");
    assert!(third.get("tools").is_none() || third["tools"].as_array().is_some_and(Vec::is_empty));
    let messages = third["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m["content"] == "FRAMEWORK B")
            .count(),
        1
    );
    assert_eq!(
        messages.iter().filter(|m| m["content"] == "ROLE B").count(),
        1
    );
    assert!(messages.iter().any(|m| m["content"] == "A finished"));
    assert!(
        !host
            .history(&caller, &target)
            .unwrap()
            .iter()
            .any(|h| matches!(h,HistoryItem::User(u) if u.input.as_text().contains("FRAMEWORK")))
    );
}

#[tokio::test]
async fn removed_referenced_model_keeps_original_input_queued_and_can_be_restored() {
    let mut server = Server::start(vec![(answer("restored"), None)]).await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let original = config(&server.endpoint);
    let host =
        Host::memory_configured(prepare(&builder, original.clone(), "old-key").await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, vec![], None);
    let mut removed = original.clone();
    removed.models.clear();
    host.reload_configuration(&caller, 0, prepare(&builder, removed, "old-key").await)
        .await
        .unwrap();
    let message_id = send(&host, &caller, &target, "accepted once");
    let state = blocked(&host, &caller, &target).await;
    assert_eq!(state.data.next_turn[0].message_id, message_id);
    assert!(host.history(&caller, &target).unwrap().is_empty());
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config.model,
        original.models[0].model
    );
    host.reload_configuration(&caller, 1, prepare(&builder, original, "old-key").await)
        .await
        .unwrap();
    // Explicitly commit the repaired selection; no accepted input is resent.
    host.update_session_async(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            model: Some("custom/provider/chat".parse().unwrap()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let request = server.request().await;
    settled(&host, &caller, &target, 1).await;
    assert_single_input(&request, &message_id, "accepted once");
}

#[tokio::test]
async fn removed_tool_and_failed_workspace_binding_do_not_consume_inputs() {
    let (builder, _) = builder_with_probe();
    let mut initial = config("http://127.0.0.1:1/v1/chat/completions");
    initial.tools = selection();
    let host =
        Host::memory_configured(prepare(&builder, initial.clone(), "old-key").await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, selection(), None);
    initial.tools.clear();
    host.reload_configuration(&caller, 0, prepare(&builder, initial, "old-key").await)
        .await
        .unwrap();
    let id = send(&host, &caller, &target, "keep for tool");
    let state = blocked(&host, &caller, &target).await;
    assert_eq!(state.data.next_turn[0].message_id, id);
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    let other = create(
        &host,
        &caller,
        vec![],
        Some(WorkspaceRef {
            path: missing.to_str().unwrap().into(),
        }),
    );
    let id = send(&host, &caller, &other, "keep for workspace");
    let state = blocked(&host, &caller, &other).await;
    assert_eq!(state.data.next_turn[0].message_id, id);
    assert!(host.history(&caller, &other).unwrap().is_empty());
}

#[tokio::test]
async fn reload_rejects_restart_invalid_stale_and_readonly_without_rotating_keys() {
    let builder = HostResourceBuilder::builtins().unwrap();
    let initial = config("http://127.0.0.1:1/v1/chat/completions");
    let host =
        Host::memory_configured(prepare(&builder, initial.clone(), "old-key").await).unwrap();
    let local = host.connect().unwrap();
    let old = host.connect_network("old-key").unwrap();
    let caller = local.caller();
    let mut restart = initial.clone();
    restart.logging.level = LogLevel::Debug;
    restart.name = "must not apply".into();
    let result = host
        .reload_configuration(&caller, 0, prepare(&builder, restart, "new-key").await)
        .await
        .unwrap();
    assert_eq!(
        result,
        HostReloadResult::RestartRequired {
            config_revision: 0,
            fields: vec![RestartField::Logging]
        }
    );
    assert_eq!(host.config_get(&caller).unwrap().config, initial);
    assert!(host.info(&old.caller()).is_ok());
    assert!(host.connect_network("new-key").is_err());
    let mut invalid = initial.clone();
    invalid.models[0].endpoint = "https://example.com/?key=private-secret".into();
    assert!(
        builder
            .prepare_with_credentials(invalid.clone(), keys(&invalid, "new-key"))
            .await
            .is_err()
    );
    assert!(host.info(&old.caller()).is_ok());
    let err = host
        .reload_configuration(
            &old.caller(),
            0,
            prepare(&builder, initial.clone(), "new-key").await,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Some(FaultKind::HostAccessReadOnly));
    let mut hot = initial.clone();
    hot.framework_instructions = "NEW FRAMEWORK".into();
    host.reload_configuration(&caller, 0, prepare(&builder, hot.clone(), "old-key").await)
        .await
        .unwrap();
    assert!(host.info(&old.caller()).is_ok());
    let err = host
        .reload_configuration(&caller, 0, prepare(&builder, initial, "new-key").await)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Some(FaultKind::HostConfigConflict));
    assert!(host.info(&old.caller()).is_ok());
    host.reload_configuration(&caller, 1, prepare(&builder, hot.clone(), "new-key").await)
        .await
        .unwrap();
    assert_eq!(
        host.info(&old.caller()).unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert!(host.info(&caller).is_ok());
    assert!(host.connect_network("new-key").is_ok());
    let public = serde_json::to_string(&host.config_get(&caller).unwrap()).unwrap();
    assert!(
        !public.contains("old-key")
            && !public.contains("new-key")
            && !public.contains("fixture-model-secret")
    );
    assert!(!format!("{:?}", prepare(&builder, hot, "new-key").await).contains("new-key"));
    let models = host.model_list(&caller).unwrap();
    assert_eq!(models[0].model.as_str(), "custom/provider/chat");
    assert_eq!(
        models[0].supported_reasoning,
        vec![ReasoningPreference::Auto]
    );
    assert!(host.tool_list(&caller).unwrap().is_empty());
}

#[tokio::test]
async fn persistent_config_uses_one_record_and_rebuilds_private_resources() {
    let directory = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let initial = config("http://127.0.0.1:1/v1/chat/completions");
    let host = Host::open_configured(directory.path(), initial.clone(), |value| async {
        Ok(prepare(&builder, value, "old-key").await)
    })
    .await
    .unwrap();
    let local = host.connect().unwrap();
    let caller = local.caller();
    let identity = host.info(&caller).unwrap();
    let mut changed = initial.clone();
    changed.name = "Saved changed".into();
    host.reload_configuration(
        &caller,
        0,
        prepare(&builder, changed.clone(), "new-key").await,
    )
    .await
    .unwrap();
    let bytes = std::fs::read(directory.path().join("host.json")).unwrap();
    let raw = String::from_utf8(bytes).unwrap();
    assert!(
        !raw.contains("old-key")
            && !raw.contains("new-key")
            && !raw.contains("fixture-model-secret")
    );
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.info(&caller).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reopened = Host::open_configured(directory.path(), initial, |value| async {
        Ok(prepare(&builder, value, "new-key").await)
    })
    .await
    .unwrap();
    let connection = reopened.connect().unwrap();
    assert_eq!(
        reopened.config_get(&connection.caller()).unwrap(),
        HostConfigSnapshot {
            config: changed,
            config_revision: 1
        }
    );
    let identity2 = reopened.info(&connection.caller()).unwrap();
    assert_eq!(identity.host_id, identity2.host_id);
    assert_ne!(identity.host_incarnation, identity2.host_incarnation);
}

#[tokio::test]
async fn owned_compaction_is_available_and_unsupported_reasoning_is_explicit() {
    let builder = HostResourceBuilder::builtins().unwrap();
    let initial = config("http://127.0.0.1:1/v1/chat/completions");
    let host = Host::memory_configured(prepare(&builder, initial, "old-key").await).unwrap();
    let connection = host.connect().unwrap();
    let mut request = request(vec![], None);
    request.config.session_defaults.execution.compaction.enabled = true;
    host.create_agent(&connection.caller(), request.clone())
        .unwrap();
    request.config.session_defaults.execution.compaction.enabled = false;
    request.config.session_defaults.reasoning = ReasoningPreference::Ultra;
    assert_eq!(
        host.create_agent(&connection.caller(), request)
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionConfigInvalid)
    );
}

#[tokio::test]
async fn model_references_and_tool_queries_use_real_available_and_owner_selected_values() {
    let (builder, _) = builder_with_probe();
    let mut initial = config("http://127.0.0.1:1/v1/chat/completions");
    initial.tools = vec![
        ToolBinding {
            name: "unselected".into(),
        },
        ToolBinding {
            name: "probe".into(),
        },
    ];
    let mut other = initial.models[0].clone();
    other.model = "another/provider/chat".parse().unwrap();
    initial.models.push(other);
    let host = Host::memory_configured(prepare(&builder, initial, "old-key").await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, selection(), None);
    let models = host.model_list(&caller).unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.model.as_str())
            .collect::<Vec<_>>(),
        ["another/provider/chat", "custom/provider/chat"]
    );
    let all = host.tool_list(&caller).unwrap();
    assert_eq!(
        all.iter()
            .map(|tool| tool.name().as_str())
            .collect::<Vec<_>>(),
        ["probe", "unselected"]
    );
    assert_eq!(
        host.tool_list_for_session(&caller, &target).unwrap(),
        vec![all[0].clone()]
    );
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id.clone(),
    };
    assert_eq!(
        host.tool_list_for_agent(&caller, &agent).unwrap(),
        vec![all[0].clone()]
    );
    host.update_session_async(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            tools: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        host.tool_list_for_session(&caller, &target)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        host.tool_list_for_agent(&caller, &agent).unwrap(),
        vec![all[0].clone()]
    );
    let public = serde_json::to_string(&(models, all)).unwrap();
    assert!(
        !public.contains("fixture-model-secret")
            && !public.contains("old-key")
            && !public.contains("endpoint")
            && !public.contains("TASK023_MODEL_KEY")
    );
}

struct RejectBinding {
    spec: ToolSpec,
}
impl ToolFactory for RejectBinding {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn bind(&self, _: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault> {
        Err(Fault::new(
            FaultKind::SessionConfigInvalid,
            "explicit fixture binding failure",
        ))
    }
}
#[tokio::test]
async fn actual_tool_binding_failure_preserves_the_accepted_message() {
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(RejectBinding {
            spec: ToolSpec::new(
                "probe".parse().unwrap(),
                "binding failure fixture",
                json!({"type":"object"}),
            )
            .unwrap(),
        }))
        .unwrap();
    let builder = HostResourceBuilder::new(Arc::new(registry));
    let mut initial = config("http://127.0.0.1:1/v1/chat/completions");
    initial.tools = selection();
    let host = Host::memory_configured(prepare(&builder, initial, "old-key").await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, selection(), None);
    let id = send(&host, &caller, &target, "original binding input");
    let state = blocked(&host, &caller, &target).await;
    assert_eq!(state.data.next_turn[0].message_id, id);
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    assert!(host.history(&caller, &target).unwrap().is_empty());
}

#[path = "task023/session_resource_resume.rs"]
mod session_resource_resume;

#[tokio::test]
async fn production_builder_binds_reload_preparer_at_initialization_without_an_extra_setter() {
    let builder = HostResourceBuilder::builtins().unwrap();
    let host =
        Host::memory_configured(builder.prepare(HostConfig::default()).await.unwrap()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut candidate = host.config_get(&caller).unwrap().config;
    candidate.name = "Assembled reload".into();
    let formed = host
        .reload(
            &caller,
            HostReloadRequest {
                operation_id: OperationId::new().unwrap(),
                expected_revision: 0,
                candidate,
            },
        )
        .unwrap();
    applied(&host, &caller, &formed.operation_id).await;
    assert_eq!(
        host.config_get(&caller).unwrap().config.name,
        "Assembled reload"
    );
}

#[path = "task023/compaction.rs"]
mod compaction;

fn assert_single_input(request: &Value, id: &MessageId, original: &str) {
    let users: Vec<_> = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .collect();
    assert_eq!(users.len(), 1, "the original queued input is consumed once");
    let (context, text) = input_context::decode(users[0]["content"].as_str().unwrap());
    assert_eq!(
        context,
        input_context::Context {
            message_id: id.clone(),
            source: MessageSource::User,
            in_reply_to: None,
        }
    );
    assert_eq!(text, original);
}
