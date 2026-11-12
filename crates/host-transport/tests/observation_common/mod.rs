#![allow(dead_code)]
//! Real owners and internal execution primitives, with explicitly scripted/gated model and file tool.
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
    tools::ToolSet,
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

pub type Events = mpsc::UnboundedSender<ModelEvent>;
pub struct ScriptedModel {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<mpsc::UnboundedReceiver<ModelEvent>>>,
    started: mpsc::UnboundedSender<()>,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let receiver = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("declared scripted round");
        self.started.send(()).unwrap();
        Box::pin(async move {
            Ok(Box::pin(stream::unfold(
                (receiver, false),
                |(mut receiver, done)| async move {
                    if done {
                        return None;
                    }
                    receiver.recv().await.map(|event| {
                        let finished = matches!(event, ModelEvent::Finish { .. });
                        (Ok(event), (receiver, finished))
                    })
                },
            )) as ModelStream)
        })
    }
}
pub struct Resources {
    pub model: Arc<dyn Model>,
    pub tool: Option<Arc<dyn minicore_host_runtime::tools::Tool>>,
}
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            let mut tools = ToolSet::builder();
            if let Some(tool) = &self.tool {
                tools.register_arc(Arc::clone(tool));
            }
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    Arc::clone(&self.model),
                    RuntimeReasoning::Auto,
                    tools.build().unwrap(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
    fn bind_tool_output(
        &self,
        execution: &ExecutionConfig,
        output: ToolOutputBinding,
    ) -> ExecutionConfig {
        let Some(tool) = &self.tool else {
            return execution.clone();
        };
        let mut tools = ToolSet::builder();
        tools.register_arc(output.wrap(Arc::clone(tool)));
        ExecutionConfig::new(
            Arc::clone(execution.model()),
            execution.reasoning(),
            tools.build().unwrap(),
            execution.policy().cloned(),
            Arc::clone(execution.prompt()),
        )
        .unwrap()
    }
}
pub fn resources(
    rounds: usize,
    tool: Option<Arc<dyn minicore_host_runtime::tools::Tool>>,
) -> (
    Arc<dyn ResourceResolver>,
    Vec<Events>,
    mpsc::UnboundedReceiver<()>,
) {
    let mut inputs = Vec::new();
    let mut streams = VecDeque::new();
    for _ in 0..rounds {
        let (send, recv) = mpsc::unbounded_channel();
        inputs.push(send);
        streams.push_back(recv);
    }
    let (started, starts) = mpsc::unbounded_channel();
    let model = Arc::new(ScriptedModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(streams),
        started,
    });
    (Arc::new(Resources { model, tool }), inputs, starts)
}

pub fn scope(target: &SessionRef) -> SubscriptionScope {
    SubscriptionScope::Session {
        agent_id: target.agent_id.clone(),
        session_id: target.session_id.clone(),
    }
}
pub fn request(scope: SubscriptionScope, categories: &[EventCategory]) -> SubscribeRequest {
    SubscribeRequest {
        scope,
        categories: categories.iter().copied().collect(),
    }
}
pub fn finish(events: &Events) {
    events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        })
        .unwrap();
}
pub async fn next(
    subscription: &mut minicore_host_transport::ClientSubscription,
) -> ObservationEvent {
    match tokio::time::timeout(std::time::Duration::from_secs(5), subscription.recv())
        .await
        .expect("gated observation timeout")
        .unwrap()
        .expect("live subscription")
    {
        ObservationNotification::Event(push) => push.event,
        control => panic!("unexpected control {control:?}"),
    }
}
pub async fn model_output(
    subscription: &mut minicore_host_transport::ClientSubscription,
) -> ObservationEvent {
    loop {
        let event = next(subscription).await;
        if matches!(
            event,
            ObservationEvent::Activity(ActivityEvent::ModelOutput { .. })
        ) {
            return event;
        }
    }
}
pub async fn turn_finished(
    subscription: &mut minicore_host_transport::ClientSubscription,
) -> TurnIdentity {
    loop {
        if let ObservationEvent::Activity(ActivityEvent::TurnFinished { source, .. }) =
            next(subscription).await
        {
            return source;
        }
    }
}

pub fn setup(
    rounds: usize,
    tool: Option<Arc<dyn minicore_host_runtime::tools::Tool>>,
) -> (Arc<Host>, Vec<Events>, mpsc::UnboundedReceiver<()>) {
    let (resources, inputs, starts) = resources(rounds, tool);
    (Host::memory(resources).unwrap(), inputs, starts)
}
