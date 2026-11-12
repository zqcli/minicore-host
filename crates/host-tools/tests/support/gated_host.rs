//! Real Host/Runtime owner fixture with an explicit gated model test double.
use futures_util::stream;
use minicore_host_core::{
    BoundResources, ConnectionHandle, Host, PreparedTurnBinding, ResourceFuture, ResourceResolver,
    TurnSource,
};
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelMessage,
        ModelRequest, ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
};
use minicore_host_tools::{AgentToolFactory, CoreTurnPort, ScopeManager, ToolRegistry};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::mpsc;

pub(crate) struct ModelCall {
    pub(crate) source: TurnSource,
    pub(crate) request: ModelRequest,
    pub(crate) events: mpsc::UnboundedSender<ModelEvent>,
}
impl ModelCall {
    pub(crate) fn complete(&self, text: &str) {
        self.events.send(ModelEvent::text_delta(text)).unwrap();
        self.events
            .send(ModelEvent::Finish {
                reason: ModelFinishReason::Stop,
            })
            .unwrap();
    }
    pub(crate) fn tool(&self, id: &str, args: Value) {
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
    pub(crate) fn last_tool_result(&self) -> Value {
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
pub(crate) struct Resources {
    pub(crate) calls: mpsc::UnboundedSender<ModelCall>,
    pub(crate) tools: ToolRegistry,
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
                options: ExecutionOptions::default(),
            })
        })
    }
}
pub(crate) struct Fixture {
    pub(crate) host: Arc<Host>,
    pub(crate) connection: ConnectionHandle,
    pub(crate) calls: mpsc::UnboundedReceiver<ModelCall>,
}
impl Fixture {
    pub(crate) fn new() -> Self {
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
    pub(crate) fn send(&self, target: &SessionRef, text: &str) -> MessageId {
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
    pub(crate) async fn next(&mut self, session: &SessionRef) -> ModelCall {
        let call = tokio::time::timeout(Duration::from_secs(5), self.calls.recv())
            .await
            .expect("actual owner must progress")
            .unwrap();
        assert_eq!(&call.source.session, session);
        call
    }
    pub(crate) async fn finished(&self, source: &TurnSource) {
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
