//! Real Host/Session/Runtime owners. Models and tools below are named test
//! doubles; no external provider, listener, or TUI validation is claimed.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet,
        ToolSpec,
    },
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

struct UnusedResources;
impl ResourceResolver for UnusedResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|e| Fault::new(FaultKind::SessionConfigInvalid, e.to_string()))
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async { panic!("this test does not execute a Turn") })
    }
}
fn request(mode: StorageMode) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.role.instructions = "TASK071_CONFIG_BODY_MUST_NOT_BE_LISTED".repeat(1024);
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "discovery Agent".into(),
        mode,
        config,
        default_session: None,
        initial_message: None,
    }
}
async fn create(host: &Arc<Host>, caller: &CallerContext, mode: StorageMode) -> SessionRef {
    let receipt = host
        .create_agent_async(caller, request(mode))
        .await
        .unwrap();
    assert!(receipt.result.steps.agent.succeeded());
    SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    }
}
async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn shutdown(host: &Arc<Host>, caller: &CallerContext) {
    host.shutdown(
        caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    until(|| host.info(caller).unwrap().lifecycle == HostLifecycle::Stopped).await;
}
fn check_no_config(value: &impl serde::Serialize) {
    let json = serde_json::to_string(value).unwrap();
    assert!(!json.contains("TASK071_CONFIG_BODY_MUST_NOT_BE_LISTED"));
    for field in [
        "\"config\":",
        "\"history\":",
        "\"arguments\":",
        "\"result\":",
    ] {
        assert!(
            !json.contains(field),
            "unexpected body field {field}: {json}"
        );
    }
}

#[tokio::test]
async fn actual_host_facilities_and_query_values_do_not_retain_resources() {
    let resources = Arc::new(UnusedResources);
    let weak = Arc::downgrade(&resources);
    let spec = ToolSpec::new(
        "fixture".parse().unwrap(),
        "actual configured declaration",
        serde_json::json!({"type":"object"}),
    )
    .unwrap();
    let config = HostConfig {
        name: "owner discovery".into(),
        models: vec![OpenAiModelSettings {
            model: "configured/model".parse().unwrap(),
            endpoint: "https://example.invalid/v1".into(),
            provider_model: "provider-model".into(),
            api_key_env: "TASK071_TEST_SECRET".into(),
            context_window: 4096,
            supported_reasoning: vec![ReasoningPreference::Auto],
            supports_tools: true,
            request_timeout_ms: None,
        }],
        tools: vec![ToolBinding {
            name: "fixture".into(),
        }],
        ..Default::default()
    };
    let host = Host::memory_configured(
        PreparedHostConfiguration::new(config, resources, vec![spec], None).unwrap(),
    )
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    let info = host.info(&caller).unwrap();
    assert_eq!(info.name, "owner discovery");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.capabilities.storage_modes, vec![StorageMode::Memory]);
    assert!(!info.capabilities.configuration_reload);
    assert_eq!((info.resources.models, info.resources.tools), (1, 1));
    assert!(!info.facilities.storage_open);
    assert!(info.facilities.execution_resources);
    assert!(!info.facilities.network_authentication);
    host.configure_network_access(Some("fixture credential".into()))
        .unwrap();
    assert!(
        host.info(&caller)
            .unwrap()
            .facilities
            .network_authentication
    );
    let held_list = host
        .list_agents(&caller, &AgentListRequest::default())
        .unwrap();
    let held_state = host.session_state(&caller, &target).unwrap();
    check_no_config(&held_list);
    check_no_config(&held_state);
    assert!(held_state.data.end_reason.is_none());
    shutdown(&host, &caller).await;
    assert!(!host.info(&caller).unwrap().facilities.execution_resources);
    assert!(
        weak.upgrade().is_none(),
        "query values must not keep a resolver alive"
    );
    assert_eq!(held_list.items.len(), 1);
    assert!(
        held_state.data.end_reason.is_none(),
        "shutdown is not business close"
    );
}

#[tokio::test]
async fn agent_pages_are_exclusive_lexical_and_none_returns_all() {
    let host = Host::memory(Arc::new(UnusedResources)).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    for _ in 0..5 {
        create(&host, &caller, StorageMode::Memory).await;
    }
    let all = host
        .list_agents(&caller, &AgentListRequest::default())
        .unwrap();
    assert_eq!(all.items.len(), 5);
    assert!(all.next_cursor.is_none());
    assert!(all.items.windows(2).all(|pair| pair[0].id < pair[1].id));
    check_no_config(&all);
    let first = host
        .list_agents(
            &caller,
            &AgentListRequest {
                cursor: None,
                limit: Some(2),
            },
        )
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.next_cursor, Some(all.items[1].id.clone()));
    let rest = host
        .list_agents(
            &caller,
            &AgentListRequest {
                cursor: first.next_cursor,
                limit: None,
            },
        )
        .unwrap();
    assert_eq!(
        rest.items.iter().map(|item| &item.id).collect::<Vec<_>>(),
        all.items[2..]
            .iter()
            .map(|item| &item.id)
            .collect::<Vec<_>>()
    );
    assert!(rest.next_cursor.is_none());
    assert_eq!(
        host.list_agents(
            &caller,
            &AgentListRequest {
                cursor: None,
                limit: Some(0)
            }
        )
        .unwrap_err()
        .kind(),
        Some(FaultKind::HostObservationInvalid)
    );
    let details = host
        .agent_details(
            &caller,
            &AgentRef {
                host_id: all.items[0].host_id.clone(),
                agent_id: all.items[0].id.clone(),
            },
        )
        .unwrap();
    assert_eq!(details.sessions.len(), 1);
    assert!(
        details
            .info
            .config
            .role
            .instructions
            .contains("TASK071_CONFIG_BODY")
    );
    shutdown(&host, &caller).await;
}

#[tokio::test]
async fn session_filters_precede_pagination_and_close_reason_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open_persistent(
        dir.path(),
        HostSettings {
            name: "persistent discovery".into(),
        },
        Arc::new(UnusedResources),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id.clone(),
    };
    for mode in [
        StorageMode::Memory,
        StorageMode::Persistent,
        StorageMode::Persistent,
        StorageMode::Memory,
    ] {
        host.create_session_async(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent.clone(),
                mode: Some(mode),
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    }
    let all = host
        .list_sessions(&caller, &agent, &SessionListRequest::default())
        .unwrap();
    assert_eq!(all.items.len(), 5);
    assert!(all.items.windows(2).all(|pair| pair[0].id < pair[1].id));
    check_no_config(&all);
    let close = all
        .items
        .iter()
        .find(|item| item.mode == StorageMode::Persistent)
        .unwrap();
    let closed = SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: close.id.clone(),
    };
    host.close_session(&caller, &closed).unwrap();
    until(|| {
        host.session_state(&caller, &closed).unwrap().data.lifecycle == SessionLifecycle::Closed
    })
    .await;
    assert_eq!(
        host.session_state(&caller, &closed)
            .unwrap()
            .data
            .end_reason
            .as_deref(),
        Some("explicit close")
    );
    let all = host.sessions(&caller, &agent).unwrap();
    let expected = all
        .iter()
        .filter(|item| {
            item.mode == StorageMode::Persistent && item.lifecycle == SessionLifecycle::Open
        })
        .collect::<Vec<_>>();
    let first = host
        .list_sessions(
            &caller,
            &agent,
            &SessionListRequest {
                mode: Some(StorageMode::Persistent),
                lifecycle: Some(SessionLifecycle::Open),
                limit: Some(1),
                cursor: None,
            },
        )
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, expected[0].id);
    assert_eq!(first.next_cursor, Some(expected[0].id.clone()));
    let rest = host
        .list_sessions(
            &caller,
            &agent,
            &SessionListRequest {
                mode: Some(StorageMode::Persistent),
                lifecycle: Some(SessionLifecycle::Open),
                limit: None,
                cursor: first.next_cursor,
            },
        )
        .unwrap();
    assert_eq!(rest.items.len(), expected.len() - 1);
    assert_eq!(rest.items[0].id, expected[1].id);
    assert!(rest.next_cursor.is_none());
    let closed_only = host
        .list_sessions(
            &caller,
            &agent,
            &SessionListRequest {
                lifecycle: Some(SessionLifecycle::Closed),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(closed_only.items.len(), 1);
    assert_eq!(
        closed_only.items[0].end_reason.as_deref(),
        Some("explicit close")
    );
    assert_eq!(
        host.info(&caller).unwrap().capabilities.storage_modes,
        vec![StorageMode::Memory, StorageMode::Persistent]
    );
    shutdown(&host, &caller).await;
    drop(connection);
    drop(host);
    let reopened = Host::open_persistent(
        dir.path(),
        HostSettings {
            name: "ignored".into(),
        },
        Arc::new(UnusedResources),
    )
    .await
    .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    let state = reopened.session_state(&caller, &closed).unwrap();
    assert_eq!(state.data.end_reason.as_deref(), Some("explicit close"));
    assert_eq!(state.data.lifecycle, SessionLifecycle::Closed);
    assert!(!state.data.owned_work.history_save);
    assert!(!state.data.owned_work.metadata_save);
    assert!(state.data.active_turn_id.is_none());
    assert!(reopened.session_state(&caller, &target).is_ok());
    shutdown(&reopened, &caller).await;
}

struct Gate {
    entered: Semaphore,
    release: Semaphore,
}
impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        })
    }
    async fn block(&self) {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
    }
    async fn reached(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
}
struct ScriptedModel {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<Vec<ModelEvent>>>,
    gate: Arc<Gate>,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let events = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted response");
        Box::pin(async move {
            self.gate.block().await;
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))) as ModelStream)
        })
    }
}
struct GatedTool {
    spec: ToolSpec,
    gate: Arc<Gate>,
}
impl Tool for GatedTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            self.gate.block().await;
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("fixture result").unwrap(),
            ))
        })
    }
}
struct CleanupGate(Arc<Gate>);
impl TurnToolCleanup for CleanupGate {
    fn finish(&self) -> OwnerFuture<'_, Vec<Fault>> {
        Box::pin(async {
            self.0.block().await;
            Ok(vec![])
        })
    }
}
struct RuntimeResources {
    model: Arc<dyn Model>,
    tools: ToolSet,
    cleanup: Arc<Gate>,
}
impl ResourceResolver for RuntimeResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    self.tools.clone(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        binding
            .register_tool_cleanup(Arc::new(CleanupGate(self.cleanup.clone())))
            .unwrap();
        self.resolve(config)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_tool_and_cleanup_work_follow_real_runtime_and_published_state() {
    let model_gate = Gate::new();
    let tool_gate = Gate::new();
    let cleanup = Gate::new();
    let model = Arc::new(ScriptedModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            4096,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(VecDeque::from([
            vec![
                ModelEvent::ToolCallStart {
                    tool_call_id: "call-071".parse().unwrap(),
                    tool_name: "fixture".parse().unwrap(),
                },
                ModelEvent::tool_call_arguments_delta("call-071".parse().unwrap(), "{}"),
                ModelEvent::ToolCallEnd {
                    tool_call_id: "call-071".parse().unwrap(),
                },
                ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                },
            ],
            vec![
                ModelEvent::text_delta("done"),
                ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                },
            ],
        ])),
        gate: model_gate.clone(),
    });
    let spec = ToolSpec::new(
        "fixture".parse().unwrap(),
        "gated fixture",
        serde_json::json!({"type":"object"}),
    )
    .unwrap();
    let mut builder = ToolSet::builder();
    builder.register(GatedTool {
        spec,
        gate: tool_gate.clone(),
    });
    let tools = builder.build().unwrap();
    let host = Host::memory(Arc::new(RuntimeResources {
        model,
        tools,
        cleanup: cleanup.clone(),
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    let mut states = host.session_states(&caller, &target).unwrap();
    host.send(
        &caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                text: "run fixture tool".into(),
                mode: MessageMode::FollowUp,
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
    model_gate.reached().await;
    until(|| states.snapshot().unwrap().data.owned_work.model).await;
    assert!(states.snapshot().unwrap().data.owned_work.tools.is_empty());
    model_gate.release.add_permits(1);
    tool_gate.reached().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = states.snapshot().unwrap();
            if state
                .data
                .owned_work
                .tools
                .iter()
                .any(|tool| tool.state == ToolCallState::Running)
            {
                break;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let state = host.session_state(&caller, &target).unwrap();
    assert_eq!(
        state.data.owned_work.tools[0].tool_call_id,
        ToolCallId("call-071".into())
    );
    check_no_config(&state);
    tool_gate.release.add_permits(1);
    model_gate.reached().await;
    model_gate.release.add_permits(1);
    cleanup.reached().await;
    let state = host.session_state(&caller, &target).unwrap();
    assert!(state.data.owned_work.tool_cleanup);
    let turn = state.data.active_turn_id.clone().unwrap();
    assert_eq!(
        host.turn_info(&caller, &target, &turn).unwrap().state,
        TurnState::Finalizing
    );
    assert!(
        state.data.end_reason.is_none(),
        "a Turn outcome does not close its Session"
    );
    cleanup.release.add_permits(1);
    until(|| {
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .active_turn_id
            .is_none()
    })
    .await;
    let state = host.session_state(&caller, &target).unwrap();
    assert!(!state.data.owned_work.tool_cleanup);
    assert!(state.data.owned_work.tools.is_empty());
    assert_eq!(state.data.lifecycle, SessionLifecycle::Open);
    assert!(state.data.end_reason.is_none());
    shutdown(&host, &caller).await;
}

#[test]
fn accepted_metadata_save_and_agent_own_deletion_remain_visible_while_disk_waits() {
    // A real runtime's single blocking slot gates real storage work. No owner
    // state is manufactured and no production save hook is added.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::open_persistent(
            dir.path(),
            HostSettings {
                name: "metadata discovery".into(),
            },
            Arc::new(UnusedResources),
        )
        .await
        .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let target = create(&host, &caller, StorageMode::Persistent).await;
        let agent = AgentRef {
            host_id: host.info(&caller).unwrap().host_id,
            agent_id: target.agent_id.clone(),
        };
        let (entered, receiver) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        receiver.await.unwrap();
        let update = {
            let host = host.clone();
            let caller = caller.clone();
            let target = target.clone();
            tokio::spawn(async move {
                host.update_session_async(
                    &caller,
                    &target,
                    0,
                    &SessionConfigPatch {
                        role: Some(Role {
                            name: "updated".into(),
                            description: String::new(),
                            instructions: "metadata save is real".into(),
                        }),
                        ..Default::default()
                    },
                )
                .await
            })
        };
        until(|| {
            host.session_state(&caller, &target)
                .unwrap()
                .data
                .owned_work
                .metadata_save
        })
        .await;
        let page = host
            .list_sessions(&caller, &agent, &SessionListRequest::default())
            .unwrap();
        assert!(page.items[0].owned_work.metadata_save);
        assert_eq!(
            page.items[0].config_revision, 0,
            "unconfirmed candidate is not committed configuration"
        );
        assert!(!update.is_finished());
        host.agent_delete(
            &caller,
            &agent,
            AgentDeleteRequest {
                operation_id: OperationId::new().unwrap(),
                scope: AgentDeleteScope::Sessions {
                    session_ids: vec![target.session_id.clone()],
                    close: true,
                },
            },
        )
        .unwrap();
        let summaries = host.agents(&caller).unwrap();
        assert_eq!(summaries[0].deletion, Some(DataDeletion::Pending {}));
        let deletion = serde_json::to_value(&summaries[0]).unwrap()["deletion"].clone();
        assert_eq!(deletion, serde_json::json!({"status":"pending"}));
        release.send(()).unwrap();
        blocker.await.unwrap();
        assert_eq!(update.await.unwrap().unwrap().config_revision, 1);
        until(|| host.agents(&caller).is_ok_and(|agents| agents.is_empty())).await;
        shutdown(&host, &caller).await;
    });
}
