//! Real Runtime AgentLoop, explicitly scripted model and owner-port fixtures.
mod support;
use futures_util::stream;
use minicore_host_protocol::ToolBinding;
use minicore_host_tools::*;
use minicore_runtime::{
    AgentLoop, ExecutionConfig, HistoryItem, LoopInputs, LoopOptions, LoopOutcome, LoopRequest,
    UserInput,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelMessage,
        ModelRequest, ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolResultOutcome,
};
use serde_json::json;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};
use support::*;

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
struct NoInputs;
impl LoopInputs for NoInputs {
    fn take(&self) -> Vec<UserInput> {
        Vec::new()
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
                )
                .unwrap(),
                ModelEvent::ToolCallEnd {
                    tool_call_id: "provider-call-42".parse().unwrap(),
                },
                ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                },
            ],
            vec![
                ModelEvent::text_delta("Accepted independent work").unwrap(),
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
    let mut runtime = AgentLoop::start(
        LoopRequest::new(
            Arc::from([]),
            UserInput::text("delegate work").unwrap(),
            config,
        ),
        LoopOptions::default_checked().unwrap(),
        Arc::new(NoInputs),
    )
    .unwrap();
    let _events = runtime.take_events().unwrap();
    let report = runtime.join().await.unwrap();
    assert_eq!(report.outcome, LoopOutcome::Completed);
    assert_eq!((report.requests, report.tool_rounds), (2, 1));
    let results = report
        .appended
        .iter()
        .filter_map(|item| match item {
            HistoryItem::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
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
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests[0].tools().len(), 1);
    assert_eq!(requests[0].tools()[0], agent_tool_spec());
    assert!(requests[1].messages().iter().any(|message|matches!(message,ModelMessage::Tool {tool_call_id,..} if tool_call_id.as_str()=="provider-call-42")));
}
