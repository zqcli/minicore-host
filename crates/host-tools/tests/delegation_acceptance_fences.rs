//! Production Host, Runtime, agent tool, owner ports and sink; gated model only.
#[path = "../../host-core/tests/support/input_context.rs"]
mod input_context;
use futures_util::stream;
use minicore_host_core::{
    BoundResources, ConnectionHandle, Host, MessageBinding, PreparedTurnBinding, ResourceFuture,
    ResourceResolver, TurnSource,
};
use minicore_host_protocol::*;
use minicore_host_tools::{AgentToolFactory, CoreTurnPort, ScopeManager, ToolRegistry};
use minicore_runtime::{
    ExecutionConfig, HistoryItem, LoopOptions,
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
        config.session_defaults.tools = vec![ToolBinding {
            name: "agent".into(),
        }];
        let receipt = self
            .host
            .create_agent(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "fence fixture".into(),
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
fn target(session: &SessionRef) -> Value {
    json!({"kind":"session", "agent_id":session.agent_id, "session_id":session.session_id})
}
#[derive(Clone, Copy)]
enum End {
    Complete,
    RemoveFirst,
    Cancel,
    Fail,
}
async fn shared_range_with_real_tools(end: End) {
    let mut f = Fixture::new();
    let parent = f.agent();
    let child = f.agent();
    f.send(&parent, "parent task");
    let root = f.next(&parent).await;
    let unrelated = f.send(&child, "older ordinary task");
    let old = f.next(&child).await;
    root.tool("delegate-first", json!({"action":"send", "target":target(&child), "text":"first follow-up", "mode":"follow_up", "delegate":true}));
    let after_first = f.next(&parent).await;
    let first_receipt = after_first.last_tool_result();
    assert_eq!(first_receipt["result"]["delivery"], "accepted");
    let first: MessageId = serde_json::from_value(first_receipt["message_id"].clone()).unwrap();
    after_first.tool("delegate-steer", json!({"action":"send", "target":target(&child), "text":"supplemental steer", "mode":"steer"}));
    let after_second = f.next(&parent).await;
    let supplemental_receipt = after_second.last_tool_result();
    assert_eq!(supplemental_receipt["result"]["delivery"], "accepted");
    let supplemental: MessageId =
        serde_json::from_value(supplemental_receipt["message_id"].clone()).unwrap();
    after_second.complete("launches accepted");
    f.finished(&root.source).await;

    // The real agent tool runs in T0. Runtime consumes M2 at the next request,
    // while the earlier follow-up M1 must remain queued for a different Turn.
    old.tool(
        "inspect-shared-child",
        json!({"action":"inspect", "target":target(&child)}),
    );
    let continuation = f.next(&child).await;
    assert_eq!(continuation.source.turn, old.source.turn);
    assert_eq!(
        continuation.last_tool_result()["session_id"],
        json!(child.session_id)
    );
    assert!(
        matches!(f.host.message(&f.connection.caller(), &child, &supplemental).unwrap().binding,
        MessageBinding::Bound { turn_id } if turn_id == old.source.turn.turn_id)
    );
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &child, &first)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    match end {
        End::RemoveFirst => {
            f.host
                .remove_input(&f.connection.caller(), &child, &first)
                .unwrap();
            continuation.complete("actual supplemental answer");
        }
        End::Complete => continuation.complete("actual supplemental answer"),
        End::Cancel | End::Fail => {
            f.host.pause(&f.connection.caller(), &child).unwrap();
            continuation
                .events
                .send(ModelEvent::text_delta(
                    "actual unfinished supplemental text",
                ))
                .unwrap();
            if matches!(end, End::Cancel) {
                f.host
                    .cancel_session(&f.connection.caller(), &child, true)
                    .unwrap();
            }
            // EOF without Finish is a real Runtime model failure, not a
            // manufactured owner result. Cancellation uses the actual handle.
            drop(continuation);
            f.finished(&old.source).await;
            let result = f
                .host
                .turn_result(&f.connection.caller(), &child, &old.source.turn.turn_id)
                .unwrap();
            assert_eq!(
                result.info.outcome,
                Some(if matches!(end, End::Cancel) {
                    TurnOutcome::Cancelled
                } else {
                    TurnOutcome::Failed
                })
            );
            assert!(f.calls.try_recv().is_err());
            assert!(f.host.session_info(&f.connection.caller(), &child).is_ok());
            assert!(matches!(
                f.host
                    .message(&f.connection.caller(), &child, &first)
                    .unwrap()
                    .binding,
                MessageBinding::Queued
            ));
            assert!(
                f.host
                    .session_state(&f.connection.caller(), &parent)
                    .unwrap()
                    .data
                    .next_turn
                    .is_empty()
            );
            f.host.resume(&f.connection.caller(), &child).unwrap();
        }
    }
    if !matches!(end, End::RemoveFirst) {
        let follow_up = f.next(&child).await;
        assert_ne!(follow_up.source.turn, old.source.turn);
        assert!(
            f.host
                .session_state(&f.connection.caller(), &parent)
                .unwrap()
                .data
                .next_turn
                .is_empty()
        );
        follow_up.complete("actual initial follow-up answer");
    }
    let notified = f.next(&parent).await;
    assert_ne!(notified.source.turn, root.source.turn);
    let notice = notified
        .request
        .messages()
        .iter()
        .filter_map(|message| {
            if let ModelMessage::User(text) = message {
                let (context, original) = input_context::decode(text);
                let notice = serde_json::from_str::<Value>(original).ok()?;
                if notice.get("task_message_id").is_some() {
                    assert_eq!(
                        context.source,
                        MessageSource::Agent {
                            source: child.clone()
                        }
                    );
                }
                Some(notice)
            } else {
                None
            }
        })
        .find(|value| value.get("task_message_id").is_some())
        .expect("one actual final notification");
    assert_eq!(notice["source"], json!(child));
    assert_eq!(notice["task_message_id"], json!(first));
    assert_eq!(notice["origin"]["tool_call_id"], "delegate-first");
    assert_eq!(notice["status"], "ended");
    assert_eq!(notice["inputs"][0]["turn"], json!(old.source.turn));
    assert_eq!(
        notice["inputs"][0]["message_ids"],
        json!([unrelated, supplemental.clone()])
    );
    assert_eq!(
        notice["inputs"][0]["delegated_message_ids"],
        json!([supplemental])
    );
    let count = if matches!(end, End::RemoveFirst) {
        1
    } else {
        2
    };
    assert_eq!(notice["results"].as_array().unwrap().len(), count);
    assert_eq!(
        notice["results"][0]["turn_id"],
        json!(old.source.turn.turn_id)
    );
    match end {
        End::Complete | End::RemoveFirst => assert_eq!(
            notice["results"][0]["output"],
            json!(["actual supplemental answer"])
        ),
        End::Cancel => assert_eq!(notice["results"][0]["outcome"], "cancelled"),
        End::Fail => assert_eq!(notice["results"][0]["outcome"], "failed"),
    }
    if matches!(end, End::RemoveFirst) {
        assert_eq!(notice["discarded_inputs"], json!([first]));
    } else {
        assert_eq!(notice["inputs"][1]["message_ids"], json!([first]));
        assert_eq!(
            notice["results"][1]["output"],
            json!(["actual initial follow-up answer"])
        );
        assert_eq!(notice["discarded_inputs"], json!([]));
    }
    let child_result = f
        .host
        .turn_result(&f.connection.caller(), &child, &old.source.turn.turn_id)
        .unwrap();
    assert!(child_result.history.iter().any(|item| matches!(item, HistoryItem::ToolResult(result) if result.call_id.as_str() == "inspect-shared-child")));
    let original = f
        .host
        .turn_result(&f.connection.caller(), &parent, &root.source.turn.turn_id)
        .unwrap();
    let saved_launch = original
        .history
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolResult(result) if result.call_id.as_str() == "delegate-first" => {
                Some(serde_json::from_str::<Value>(result.output.content().as_str()).unwrap())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(saved_launch, first_receipt);
    notified.complete("received one real notification");
    f.finished(&notified.source).await;
    assert!(f.calls.try_recv().is_err());
    assert_eq!(
        f.host.turns(&f.connection.caller(), &parent).unwrap().len(),
        2
    );
    // Reused ordinary memory children remain available until explicitly closed.
    assert!(f.host.session_info(&f.connection.caller(), &child).is_ok());
    f.host
        .close_session(&f.connection.caller(), &child)
        .unwrap();
    assert!(f.host.session_info(&f.connection.caller(), &child).is_err());
}
#[tokio::test]
async fn real_agent_tool_notifies_both_shared_turns_in_actual_order() {
    shared_range_with_real_tools(End::Complete).await;
}
#[tokio::test]
async fn real_agent_tool_accepts_supplemental_result_after_first_input_removed() {
    shared_range_with_real_tools(End::RemoveFirst).await;
}
#[tokio::test]
async fn real_agent_tool_retains_failed_shared_turn_until_preserved_inbox_settles() {
    shared_range_with_real_tools(End::Fail).await;
}
#[tokio::test]
async fn real_agent_tool_retains_cancelled_shared_turn_until_preserved_inbox_settles() {
    shared_range_with_real_tools(End::Cancel).await;
}
