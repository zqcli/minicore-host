//! Actual model/tool primitives with explicitly scripted model and owner-port fixtures.
mod support;
use futures_util::stream;
use minicore_host_protocol::ToolBinding;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions, InteractionAnswer, InteractionId, LoopId,
    PendingInteraction,
    model::{
        AssistantPart, Model, ModelCallContext, ModelDescriptor, ModelDriver, ModelDriverConfig,
        ModelEvent, ModelFinishReason, ModelMessage, ModelRequest, ModelStartFuture, ModelStream,
        ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::{
        ToolBatchContext, ToolBatchEvent, ToolBatchHost, ToolBatchStop, ToolResultOutcome,
        run_tool_batch,
    },
};
use minicore_host_tools::*;
use serde_json::json;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use support::*;
use tokio::sync::oneshot;

struct FixtureModel {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<Vec<ModelEvent>>>,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Model for FixtureModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.requests.lock().unwrap().push(request);
        let events = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("explicit fixture round");
        Box::pin(
            async move { Ok(Box::pin(stream::iter(events.into_iter().map(Ok))) as ModelStream) },
        )
    }
}
struct NoInteractions;
impl ToolBatchHost for NoInteractions {
    fn observe(&self, _: ToolBatchEvent) {}
    fn request_interaction(
        &self,
        _: PendingInteraction,
        _: oneshot::Sender<InteractionAnswer>,
    ) -> Result<(), ToolBatchStop> {
        panic!("agent acceptance requires no interaction")
    }
    fn finish_interaction(&self, _: InteractionId) {
        panic!("no interaction was registered")
    }
}
#[tokio::test]
async fn runtime_pairs_agent_acceptance_with_actual_upstream_call_id() {
    let port = FixtureTurn::new();
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
        .unwrap();
    let selected = registry
        .bind(
            &[ToolBinding {
                name: "agent".into(),
            }],
            port.port(),
        )
        .unwrap();
    let call = "provider-call-42".parse().unwrap();
    let arguments = json!({"action":"create","resource":"session","target":{"host_id":port.source.host_id,"agent_id":port.source.session.agent_id},"text":"review"});
    let model = Arc::new(FixtureModel {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(VecDeque::from([
            vec![
                ModelEvent::ToolCallStart {
                    tool_call_id: call,
                    tool_name: "agent".parse().unwrap(),
                },
                ModelEvent::tool_call_arguments_delta(
                    "provider-call-42".parse().unwrap(),
                    arguments.to_string(),
                ),
                ModelEvent::ToolCallEnd {
                    tool_call_id: "provider-call-42".parse().unwrap(),
                },
                ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                },
            ],
            vec![
                ModelEvent::text_delta("Accepted independent work"),
                ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                },
            ],
        ])),
        requests: Mutex::new(Vec::new()),
    });
    let config = ExecutionConfig::new(
        model.clone(),
        ReasoningPreference::Auto,
        selected,
        None,
        Arc::new(DefaultPromptProvider::new(None)),
    )
    .unwrap();
    let options = ExecutionOptions::default();
    let context = ModelCallContext::new(
        LoopId::new().unwrap(),
        0,
        Default::default(),
        Instant::now() + Duration::from_secs(5),
    );
    let driver = ModelDriver::new(
        model.as_ref(),
        ModelDriverConfig::new(options.model_timeout, 1, options.model_retry_base_delay),
    )
    .unwrap();
    let mut messages = vec![ModelMessage::user("delegate work").unwrap()];
    let response = driver
        .call(
            config.request(messages.clone()).unwrap(),
            context.clone(),
            &mut |_| {},
        )
        .await
        .outcome
        .unwrap();
    assert_eq!(response.finish_reason(), ModelFinishReason::ToolCalls);
    let calls = response
        .parts()
        .iter()
        .filter_map(|part| match part {
            AssistantPart::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect::<Vec<_>>();
    let report = run_tool_batch(
        &config,
        &calls,
        ToolBatchContext {
            loop_id: context.loop_id,
            request_index: 0,
            cancellation: context.cancellation.clone(),
            deadline: Some(context.deadline),
            policy_timeout: options.policy_timeout,
            tool_timeout: options.tool_timeout,
        },
        Arc::new(NoInteractions),
    )
    .await;
    assert_eq!(report.stop, None);
    let results = report.results;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].call_id.as_str(), "provider-call-42");
    assert_eq!(results[0].outcome, ToolResultOutcome::Success);
    let acceptance: serde_json::Value =
        serde_json::from_str(results[0].output.content().as_str()).unwrap();
    assert_eq!(acceptance["result"]["delivery"], "accepted");
    assert!(acceptance["result"].get("outcome").is_none());
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    assert_eq!(
        relation.registration.origin.tool_call_id.as_str(),
        "provider-call-42"
    );
    assert_eq!(
        relation.registration.origin.turn_id,
        port.source.turn.turn_id
    );
    messages.push(
        ModelMessage::assistant_with_provider_replay(
            response.parts().to_vec(),
            response.provider_replay().cloned(),
        )
        .unwrap(),
    );
    messages.extend(results.into_iter().map(|result| {
        ModelMessage::tool_with_outcome(result.call_id, result.output, result.outcome).unwrap()
    }));
    let mut second_context = context;
    second_context.request_index = 1;
    let second = driver
        .call(
            config.request(messages).unwrap(),
            second_context,
            &mut |_| {},
        )
        .await
        .outcome
        .unwrap();
    assert_eq!(second.finish_reason(), ModelFinishReason::Stop);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].tools().len(), 1);
    assert_eq!(requests[0].tools()[0], agent_tool_spec());
    assert!(requests[1].messages().iter().any(|message|matches!(message,ModelMessage::Tool {tool_call_id,..} if tool_call_id.as_str()=="provider-call-42")));
}
