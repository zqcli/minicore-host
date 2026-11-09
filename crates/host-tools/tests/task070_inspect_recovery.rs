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
