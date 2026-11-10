//! New recovery probes using real Host/Runtime and agent-tool execution.
//! The model stream is an explicit test fixture. No capability or owner is faked.
use futures_util::stream;
use minicore_host_core::{
    BoundResources, ConnectionHandle, Host, PreparedTurnBinding, ResourceFuture, ResourceResolver,
    TurnSource,
};
use minicore_host_protocol::*;
use minicore_host_tools::{AgentToolFactory, CoreTurnPort, ScopeManager, ToolRegistry};
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelMessage,
        ModelRequest, ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::mpsc;

struct ModelCall {
    source: TurnSource,
    request: ModelRequest,
    events: mpsc::UnboundedSender<ModelEvent>,
}
impl ModelCall {
    fn complete(&self, text: &str) {
        self.events.send(ModelEvent::text_delta(text)).unwrap();
        self.events
            .send(ModelEvent::Finish {
                reason: ModelFinishReason::Stop,
            })
            .unwrap();
    }
    fn tool(&self, id: &str, args: Value) {
        self.events
            .send(ModelEvent::ToolCallStart {
                tool_call_id: id.parse().unwrap(),
                tool_name: "agent".parse().unwrap(),
            })
            .unwrap();
        self.events
            .send(ModelEvent::tool_call_arguments_delta(
                id.parse().unwrap(),
                args.to_string(),
            ))
            .unwrap();
        self.events
            .send(ModelEvent::ToolCallEnd {
                tool_call_id: id.parse().unwrap(),
            })
            .unwrap();
        self.events
            .send(ModelEvent::Finish {
                reason: ModelFinishReason::ToolCalls,
            })
            .unwrap();
    }
    fn last_tool_result(&self) -> Value {
        self.request
            .messages()
            .iter()
            .rev()
            .find_map(|message| {
                if let ModelMessage::Tool { output, .. } = message {
                    Some(serde_json::from_str(output.content().as_str()).unwrap())
                } else {
                    None
                }
            })
            .expect("actual Runtime ToolResult")
    }
}
struct GatedModel {
    descriptor: ModelDescriptor,
    source: TurnSource,
    calls: mpsc::UnboundedSender<ModelCall>,
}
impl Model for GatedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let (events, rx) = mpsc::unbounded_channel();
        self.calls
            .send(ModelCall {
                source: self.source.clone(),
                request,
                events,
            })
            .unwrap();
        Box::pin(async move {
            Ok(
                Box::pin(stream::unfold((rx, false), |(mut rx, done)| async move {
                    if done {
                        None
                    } else {
                        rx.recv().await.map(|event| {
                            let done = matches!(event, ModelEvent::Finish { .. });
                            (Ok(event), (rx, done))
                        })
                    }
                })) as ModelStream,
            )
        })
    }
}
struct Resources {
    calls: mpsc::UnboundedSender<ModelCall>,
    tools: ToolRegistry,
}
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        panic!("real prepared Turn binding required")
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(async move {
            let model = Arc::new(GatedModel {
                descriptor: ModelDescriptor::new(
                    "fixture/model".parse().unwrap(),
                    128000,
                    BTreeSet::from([ReasoningPreference::Auto]),
                    true,
                )
                .unwrap(),
                source: binding.source().clone(),
                calls: self.calls.clone(),
            });
            let tools = self
                .tools
                .bind(&config.tools, CoreTurnPort::prepared(binding))?;
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    model,
                    ReasoningPreference::Auto,
                    tools,
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
struct Fixture {
    host: Arc<Host>,
    connection: ConnectionHandle,
    calls: mpsc::UnboundedReceiver<ModelCall>,
}
impl Fixture {
    fn new() -> Self {
        let (calls, called) = mpsc::unbounded_channel();
        let mut tools = ToolRegistry::new();
        tools
            .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
            .unwrap();
        let host = Host::memory(Arc::new(Resources { calls, tools })).unwrap();
        let connection = host.connect().unwrap();
        Self {
            host,
            connection,
            calls: called,
        }
    }
    async fn open(path: &std::path::Path) -> Self {
        let (calls, called) = mpsc::unbounded_channel();
        let mut tools = ToolRegistry::new();
        tools
            .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
            .unwrap();
        let host = Host::open_persistent(
            path,
            minicore_host_core::HostSettings {
                name: "persistent inspect fixture".into(),
            },
            Arc::new(Resources { calls, tools }),
        )
        .await
        .unwrap();
        let connection = host.connect().unwrap();
        Self {
            host,
            connection,
            calls: called,
        }
    }
    async fn persistent_agent(&self) -> SessionRef {
        let mut config = AgentConfig::default();
        config.role.instructions = "original bound instructions".into();
        config.session_defaults.tools = vec![ToolBinding {
            name: "agent".into(),
        }];
        config.session_defaults.execution.compaction.enabled = false;
        let receipt = self
            .host
            .create_agent_async(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "persistent inspect fixture".into(),
                    mode: StorageMode::Persistent,
                    config,
                    default_session: None,
                    initial_message: None,
                },
            )
            .await
            .unwrap();
        SessionRef {
            agent_id: receipt.agent_id.unwrap(),
            session_id: receipt.session_id.unwrap(),
        }
    }
    async fn stop(&self) {
        self.host
            .shutdown(
                &self.connection.caller(),
                HostShutdownRequest {
                    operation_id: OperationId::new().unwrap(),
                    strategy: HostShutdownStrategy::Graceful,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.host.info(&self.connection.caller()).unwrap().lifecycle
                != HostLifecycle::Stopped
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        self.connection.close();
    }
    fn agent(&self) -> SessionRef {
        let mut config = AgentConfig::default();
        config.role.instructions = "original bound instructions".into();
        config.session_defaults.tools = vec![ToolBinding {
            name: "agent".into(),
        }];
        let receipt = self
            .host
            .create_agent(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "inspect recovery fixture".into(),
                    mode: StorageMode::Memory,
                    config,
                    default_session: None,
                    initial_message: None,
                },
            )
            .unwrap();
        SessionRef {
            agent_id: receipt.agent_id.unwrap(),
            session_id: receipt.session_id.unwrap(),
        }
    }
    fn send(&self, target: &SessionRef, text: &str) -> MessageId {
        let message = MessageInput {
            message_id: MessageId::new().unwrap(),
            mode: MessageMode::FollowUp,
            text: text.into(),
            in_reply_to: None,
        };
        self.host
            .send(
                &self.connection.caller(),
                SendMessage::new(message.clone(), target.clone()),
            )
            .unwrap();
        message.message_id
    }
    async fn next(&mut self, session: &SessionRef) -> ModelCall {
        let call = tokio::time::timeout(Duration::from_secs(5), self.calls.recv())
            .await
            .expect("actual owner must progress")
            .unwrap();
        assert_eq!(&call.source.session, session);
        call
    }
    async fn finished(&self, source: &TurnSource) {
        self.host
            .wait_turn(
                &self.connection.caller(),
                &source.session,
                &source.turn.turn_id,
            )
            .await
            .unwrap();
    }
}
fn inspect(source: &TurnSource, output: bool) -> Value {
    json!({
        "action": "inspect",
        "target": {
            "kind": "turn",
            "agent_id": source.session.agent_id,
            "session_id": source.session.session_id,
            "turn_id": source.turn.turn_id,
        },
        "include_output": output,
    })
}
fn assert_tool_summary(value: &Value, source: &TurnSource, state: &str) {
    assert_eq!(value["turn_id"], json!(source.turn.turn_id));
    assert_eq!(value["session_id"], json!(source.session.session_id));
    assert_eq!(value["data"]["state"], state);
    assert!(
        value["data"].get("config").is_none(),
        "Turn tool inspect leaked the full bound config: {value}"
    );
    assert!(!value.to_string().contains("original bound instructions"));
}
fn assert_public_binding(f: &Fixture, source: &TurnSource) {
    let detailed = f
        .host
        .turn_snapshot(
            &f.connection.caller(),
            &source.session,
            &source.turn.turn_id,
        )
        .unwrap();
    assert_eq!(
        detailed.data.config.role.instructions,
        "original bound instructions"
    );
    assert_eq!(detailed.data.state.config_revision, 0);
}

#[tokio::test]
async fn active_turn_inspect_is_lightweight_while_public_get_keeps_bound_configuration() {
    let mut f = Fixture::new();
    let session = f.agent();
    f.send(&session, "inspect the active Turn");
    let first = f.next(&session).await;
    f.host
        .update_session(
            &f.connection.caller(),
            &session,
            0,
            &SessionConfigPatch {
                role: Some(Role {
                    instructions: "next Turn instructions".into(),
                    ..Role::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    first.tool("inspect-active", inspect(&first.source, false));
    let next = f.next(&session).await;
    assert_tool_summary(&next.last_tool_result(), &first.source, "running");
    assert_public_binding(&f, &first.source);
    next.events
        .send(ModelEvent::text_delta("visible while running"))
        .unwrap();
    let mut range = inspect(&first.source, true);
    range["response_index"] = json!(1);
    range["output_range"] = json!({"channel":"text","start":0,"end":21,"max_bytes":21});
    next.tool("inspect-current-range", range);
    let next = f.next(&session).await;
    let ranged = next.last_tool_result();
    assert_tool_summary(&ranged, &first.source, "running");
    assert_eq!(ranged["data"]["output"]["text"], "visible while running");
    assert_eq!(ranged["data"]["response_index"], 1);
    assert!(ranged["data"].get("history").is_none());
    next.complete("actual first result");
    f.finished(&first.source).await;
    assert_public_binding(&f, &first.source);
}

#[tokio::test]
async fn completed_turn_inspect_omits_config_with_and_without_actual_output() {
    let mut f = Fixture::new();
    let session = f.agent();
    f.send(&session, "complete the original Turn");
    let first = f.next(&session).await;
    first.complete("actual retained completion");
    f.finished(&first.source).await;
    f.send(&session, "inspect the original completion");
    let next = f.next(&session).await;
    next.tool("inspect-completed", inspect(&first.source, false));
    let next = f.next(&session).await;
    let summary = next.last_tool_result();
    assert_tool_summary(&summary, &first.source, "finished");
    assert!(summary["data"].get("history").is_none());
    next.tool("inspect-completed-output", inspect(&first.source, true));
    let next = f.next(&session).await;
    let with_output = next.last_tool_result();
    assert_tool_summary(&with_output, &first.source, "finished");
    assert!(with_output["data"]["history"].is_array());
    assert!(
        with_output["data"]["history"]
            .to_string()
            .contains("actual retained completion")
    );
    assert_eq!(with_output["data"]["partials"], json!([]));
    assert_public_binding(&f, &first.source);
    next.complete("inspections complete");
    f.finished(&next.source).await;
}

#[tokio::test]
async fn agent_inspect_reads_saved_turn_after_restart_without_inventing_execution_metadata() {
    let path = std::env::temp_dir().join(format!(
        "minicore-inspect-saved-{}",
        OperationId::new().unwrap()
    ));
    let mut f = Fixture::open(&path).await;
    let target = f.persistent_agent().await;
    f.send(&target, "save the original answer");
    let first = f.next(&target).await;
    first.complete("actual saved answer — 原文完整保留");
    f.finished(&first.source).await;
    let old_source = first.source.clone();
    f.stop().await;
    drop(f);

    let mut f = Fixture::open(&path).await;
    let public = f
        .host
        .turn_result_snapshot(&f.connection.caller(), &target, &old_source.turn.turn_id)
        .await
        .unwrap();
    assert_ne!(
        public.source.session.host_incarnation,
        old_source.host_incarnation
    );
    assert!(public.source.load_id.is_none());
    assert!(public.data.info.is_none());
    assert!(public.data.config.is_none());
    assert_eq!(
        public.data.saved_terminal.as_ref().unwrap().outcome,
        TurnOutcome::Completed
    );
    assert!(
        f.host
            .turns(&f.connection.caller(), &target)
            .unwrap()
            .is_empty()
    );

    f.send(&target, "inspect the original saved turn");
    let current = f.next(&target).await;
    current.tool("inspect-saved-turn", inspect(&old_source, true));
    let after_tool = f.next(&target).await;
    let actual = after_tool.last_tool_result();
    assert_eq!(actual["turn_id"], json!(old_source.turn.turn_id));
    assert_eq!(
        actual["host_incarnation"],
        json!(public.source.session.host_incarnation)
    );
    assert_eq!(
        actual["data"]["history"],
        serde_json::to_value(public.data.history).unwrap()
    );
    assert!(
        actual["data"]["history"]
            .to_string()
            .contains("actual saved answer — 原文完整保留")
    );
    assert_eq!(
        actual["data"]["saved_terminal"],
        serde_json::to_value(public.data.saved_terminal.unwrap()).unwrap()
    );
    assert_eq!(actual["data"]["partials"], json!([]));
    for field in [
        "info",
        "state",
        "config",
        "load_id",
        "config_revision",
        "runner_revision",
    ] {
        assert!(
            actual["data"].get(field).is_none(),
            "invented saved execution metadata: {actual}"
        );
    }
    assert!(!actual.to_string().contains("original bound instructions"));
    after_tool.complete("inspection complete");
    f.finished(&after_tool.source).await;
    f.stop().await;
    drop(f);
    std::fs::remove_dir_all(path).unwrap();
}
