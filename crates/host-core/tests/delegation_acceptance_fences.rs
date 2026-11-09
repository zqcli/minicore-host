//! Real Host/Session/Runtime owners; only model streams are gated test fixtures.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelRequest, ModelStartFuture,
        ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::mpsc;

struct ModelCall {
    events: mpsc::UnboundedSender<ModelEvent>,
}
struct GatedModel {
    descriptor: ModelDescriptor,
    calls: mpsc::UnboundedSender<ModelCall>,
}
impl Model for GatedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let (events, rx) = mpsc::unbounded_channel();
        self.calls.send(ModelCall { events }).unwrap();
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
    model: Arc<GatedModel>,
    bindings: mpsc::UnboundedSender<PreparedTurnBinding>,
}
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        self.bindings.send(binding).unwrap();
        self.resolve(config)
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    host: Arc<Host>,
    connection: ConnectionHandle,
    bindings: mpsc::UnboundedReceiver<PreparedTurnBinding>,
    calls: mpsc::UnboundedReceiver<ModelCall>,
}
impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (calls, called) = mpsc::unbounded_channel();
        let (bindings, bound) = mpsc::unbounded_channel();
        let resources = Arc::new(Resources {
            model: Arc::new(GatedModel {
                descriptor: ModelDescriptor::new(
                    "fixture/model".parse().unwrap(),
                    128000,
                    BTreeSet::from([ReasoningPreference::Auto]),
                    true,
                )
                .unwrap(),
                calls,
            }),
            bindings,
        });
        let host = Host::open_persistent(
            directory.path(),
            HostSettings {
                name: "acceptance fences".into(),
            },
            resources,
        )
        .await
        .unwrap();
        let connection = host.connect().unwrap();
        Self {
            directory,
            host,
            connection,
            bindings: bound,
            calls: called,
        }
    }
    async fn agent(&self, mode: StorageMode) -> SessionRef {
        let receipt = self
            .host
            .create_agent_async(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "fence fixture".into(),
                    mode,
                    config: AgentConfig::default(),
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
    fn send(&self, target: &SessionRef, message: MessageInput) {
        self.host
            .send(
                &self.connection.caller(),
                SendMessage::new(message, target.clone()),
            )
            .unwrap();
    }
}
fn input(mode: MessageMode) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode,
        text: "delegated fixture input".into(),
        in_reply_to: None,
    }
}
async fn recv<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("owner must progress")
        .expect("sender alive")
}
struct Hooks;
impl ScopeHooks for Hooks {
    fn seal(&self) {}
    fn close_members(&self) -> OwnerFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn root_closed(&self) {}
}
struct Sink(mpsc::UnboundedSender<DelegationCompletion>);
impl DelegationSink for Sink {
    fn handoff(&self, completion: DelegationCompletion) -> Result<(), Fault> {
        self.0.send(completion).unwrap();
        Ok(())
    }
}
fn registration(
    cap: &TurnCapability,
    child: &SessionRef,
    first: &MessageInput,
) -> DelegationRegistration {
    let s = cap.source();
    DelegationRegistration {
        host_id: s.host_id.clone(),
        host_incarnation: s.host_incarnation.clone(),
        root: s.session.clone(),
        parent: s.session.clone(),
        child: child.clone(),
        task_message_id: first.message_id.clone(),
        origin: ToolCallRef {
            turn_id: s.turn.turn_id.clone(),
            tool_call_id: ToolCallId("bound-fixture-call".into()),
        },
        temporary: false,
    }
}
async fn explicit_load_settles_discarded_input(fail: bool, close: bool) {
    let mut f = Fixture::new().await;
    let parent = f.agent(StorageMode::Memory).await;
    let child = f
        .agent(if fail {
            StorageMode::Persistent
        } else {
            StorageMode::Memory
        })
        .await;
    f.send(&parent, input(MessageMode::FollowUp));
    let cap = recv(&mut f.bindings).await.capability();
    let parent_call = recv(&mut f.calls).await;
    assert!(!parent_call.events.is_closed());
    cap.prepare_scope(Arc::new(Hooks)).unwrap();
    f.host.pause(&f.connection.caller(), &child).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, &child, &task);
    let (sink, mut completions) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task.clone()).unwrap();
    let history = f
        .directory
        .path()
        .join("agents")
        .join(child.agent_id.to_string())
        .join("sessions")
        .join(child.session_id.to_string())
        .join("history.jsonl");
    if fail {
        std::fs::write(&history, b"invalid history\n").unwrap();
    }

    // The current-thread runtime cannot run this accepted load until the next
    // await. Close/removal therefore deterministically happens during loading.
    f.host.load_session(&f.connection.caller(), &child).unwrap();
    let states = f
        .host
        .session_states(&f.connection.caller(), &child)
        .unwrap();
    assert!(states.snapshot().unwrap().data.preparing);
    if close {
        let receipt = f
            .host
            .session_close(
                &f.connection.caller(),
                &child,
                SessionCloseRequest {
                    operation_id: OperationId::new().unwrap(),
                },
            )
            .unwrap();
        assert_eq!(receipt.result.lifecycle, SessionCloseLifecycle::Closing);
    } else {
        f.host
            .remove_input(&f.connection.caller(), &child, &task.message_id)
            .unwrap();
    }
    assert!(completions.try_recv().is_err());
    let completion = recv(&mut completions).await;
    assert!(completion.results.is_empty());
    assert!(completion.inputs.is_empty());
    assert_eq!(completion.discarded_inputs, vec![task.message_id.clone()]);
    if !close {
        let state = states.snapshot().unwrap().data;
        assert!(!state.preparing);
        assert_eq!(state.scheduling, Scheduling::Paused);
        assert_eq!(state.block_reason.is_some(), fail);
        assert!(matches!(
            f.host
                .message(&f.connection.caller(), &child, &task.message_id)
                .unwrap()
                .binding,
            MessageBinding::Removed
        ));
    }
    permit.close(OperationId::new().unwrap()).await.unwrap();
    if fail {
        assert_eq!(std::fs::read(&history).unwrap(), b"invalid history\n");
        assert_eq!(
            states.snapshot().unwrap().data.lifecycle,
            SessionLifecycle::Closed
        );
    } else {
        assert!(f.host.session_info(&f.connection.caller(), &child).is_err());
        assert!(
            f.host
                .agents(&f.connection.caller())
                .unwrap()
                .iter()
                .all(|agent| agent.id != child.agent_id)
        );
    }
    let receipt = permit.notify(input(MessageMode::FollowUp)).await.unwrap();
    assert_eq!(receipt.result.queue, InputQueue::NextStep);
    assert!(permit.notify(input(MessageMode::FollowUp)).await.is_err());
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    assert!(completions.try_recv().is_err());
    assert!(f.calls.try_recv().is_err());
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
    f.host
        .wait_turn(&f.connection.caller(), &parent, &cap.source().turn.turn_id)
        .await
        .unwrap();
}
#[tokio::test]
async fn successful_explicit_load_after_close_hands_off_before_memory_reclamation() {
    explicit_load_settles_discarded_input(false, true).await;
}
#[tokio::test]
async fn successful_explicit_load_after_remove_hands_off_while_paused() {
    explicit_load_settles_discarded_input(false, false).await;
}
#[tokio::test]
async fn failed_explicit_load_after_close_hands_off_real_discarded_input() {
    explicit_load_settles_discarded_input(true, true).await;
}
#[tokio::test]
async fn failed_explicit_load_after_remove_hands_off_without_clearing_the_fault() {
    explicit_load_settles_discarded_input(true, false).await;
}

fn complete(call: &ModelCall, text: &str) {
    call.events.send(ModelEvent::text_delta(text)).unwrap();
    call.events
        .send(ModelEvent::Finish {
            reason: minicore_runtime::model::ModelFinishReason::Stop,
        })
        .unwrap();
}
async fn appended_steer_precedes_first_follow_up(remove_first: bool) {
    let mut f = Fixture::new().await;
    let parent = f.agent(StorageMode::Memory).await;
    let child = f.agent(StorageMode::Memory).await;
    f.send(&parent, input(MessageMode::FollowUp));
    let cap = recv(&mut f.bindings).await.capability();
    let _parent_call = recv(&mut f.calls).await;
    cap.prepare_scope(Arc::new(Hooks)).unwrap();
    let unrelated = input(MessageMode::FollowUp);
    f.send(&child, unrelated.clone());
    let old = recv(&mut f.bindings).await;
    let old_call = recv(&mut f.calls).await;
    let first = input(MessageMode::FollowUp);
    let relation = registration(&cap, &child, &first);
    let (sink, mut completions) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, first.clone()).unwrap();
    let supplemental = input(MessageMode::Steer);
    cap.send_delegated(&relation, supplemental.clone()).unwrap();
    complete(&old_call, "old shared response");
    let continuation = recv(&mut f.calls).await;
    assert!(
        matches!(f.host.message(&f.connection.caller(), &child, &supplemental.message_id).unwrap().binding,
        MessageBinding::Bound { turn_id } if turn_id == old.source().turn.turn_id)
    );
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &child, &first.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    if remove_first {
        f.host
            .remove_input(&f.connection.caller(), &child, &first.message_id)
            .unwrap();
    }
    complete(&continuation, "real supplemental result");
    if !remove_first {
        let next = recv(&mut f.bindings).await;
        assert_ne!(next.source().turn, old.source().turn);
        let follow_up = recv(&mut f.calls).await;
        assert!(completions.try_recv().is_err());
        complete(&follow_up, "real first-message result");
    }
    let completion = recv(&mut completions).await;
    assert_eq!(completion.results.len(), if remove_first { 1 } else { 2 });
    assert_eq!(completion.inputs[0].turn, old.source().turn);
    assert_eq!(
        completion.inputs[0].message_ids,
        vec![unrelated.message_id, supplemental.message_id.clone()]
    );
    assert_eq!(
        completion.inputs[0].delegated_message_ids,
        vec![supplemental.message_id]
    );
    let output = completion.results[0]
        .history
        .iter()
        .filter_map(|item| {
            if let minicore_runtime::HistoryItem::Assistant(response) = item {
                Some(
                    response
                        .content
                        .iter()
                        .filter_map(|part| {
                            if let minicore_runtime::model::AssistantPart::Text(text) = part {
                                Some(text.as_ref())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        })
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(
        output,
        vec!["old shared response", "real supplemental result"]
    );
    if remove_first {
        assert_eq!(completion.discarded_inputs, vec![first.message_id]);
    } else {
        assert_eq!(
            completion.inputs[1].message_ids,
            vec![first.message_id.clone()]
        );
        assert_eq!(
            completion.inputs[1].delegated_message_ids,
            vec![first.message_id]
        );
        assert!(completion.discarded_inputs.is_empty());
    }
    let receipt = permit.notify(input(MessageMode::FollowUp)).await.unwrap();
    assert!(permit.notify(input(MessageMode::FollowUp)).await.is_err());
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    assert!(completions.try_recv().is_err());
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
    f.host
        .wait_turn(&f.connection.caller(), &parent, &cap.source().turn.turn_id)
        .await
        .unwrap();
}
#[tokio::test]
async fn appended_steer_retains_shared_turn_before_first_follow_up() {
    appended_steer_precedes_first_follow_up(false).await;
}
#[tokio::test]
async fn appended_steer_result_survives_removal_of_first_follow_up() {
    appended_steer_precedes_first_follow_up(true).await;
}
