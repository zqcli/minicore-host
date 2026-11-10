//! Actual Host -> Agent -> Session -> Runtime ownership; only models are gated fixtures.
use futures_util::{FutureExt, stream};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

type Events = mpsc::UnboundedSender<ModelEvent>;
struct ModelFixture {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<mpsc::UnboundedReceiver<ModelEvent>>>,
    starts: mpsc::UnboundedSender<()>,
}
impl Model for ModelFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let receiver = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("declared model round");
        self.starts.send(()).unwrap();
        Box::pin(async move {
            Ok(Box::pin(stream::unfold(
                (receiver, false),
                |(mut receiver, done)| async move {
                    if done {
                        None
                    } else {
                        receiver.recv().await.map(|event| {
                            let done = matches!(event, ModelEvent::Finish { .. });
                            (Ok(event), (receiver, done))
                        })
                    }
                },
            )) as ModelStream)
        })
    }
}
struct Resources {
    model: Arc<dyn Model>,
    bindings: mpsc::UnboundedSender<PreparedTurnBinding>,
    prepare_gate: Option<Arc<Semaphore>>,
    fail: AtomicBool,
    plan: Option<Arc<FileDelegationPlan>>,
    claim_gate: Mutex<Option<(mpsc::UnboundedSender<()>, Arc<std::sync::Barrier>)>>,
}
impl ResourceResolver for Resources {
    fn bind_tool_output(
        &self,
        execution: &ExecutionConfig,
        _: ToolOutputBinding,
    ) -> ExecutionConfig {
        if let Some((entered, gate)) = self.claim_gate.lock().unwrap().take() {
            entered.send(()).unwrap();
            gate.wait();
        }
        execution.clone()
    }
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        panic!("Turn preparation must use its candidate binding")
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        self.bindings.send(binding.clone()).unwrap();
        Box::pin(async move {
            assert!(binding.capability().check_active().is_err());
            if let Some(gate) = &self.prepare_gate {
                gate.acquire().await.unwrap().forget();
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "gated binding failure",
                ));
            }
            let mut tools = ToolSet::builder();
            if config
                .tools
                .iter()
                .any(|tool| tool.name == "delegate_fixture")
            {
                let plan = self.plan.as_ref().expect("registered fixture tool");
                tools.register_arc(Arc::new(FileDelegatingTool {
                    spec: minicore_runtime::tools::ToolSpec::new(
                        "delegate_fixture".parse().unwrap(),
                        "Delegate a real file task",
                        serde_json::json!({"type":"object"}),
                    )
                    .unwrap(),
                    cap: binding.capability(),
                    plan: Arc::clone(plan),
                }));
            }
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    Arc::clone(&self.model),
                    ReasoningPreference::Auto,
                    tools.build().unwrap(),
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
    resources: Arc<Resources>,
    host: Arc<Host>,
    connection: ConnectionHandle,
    events: Vec<Events>,
    starts: mpsc::UnboundedReceiver<()>,
    bindings: mpsc::UnboundedReceiver<PreparedTurnBinding>,
}
fn setup(rounds: usize, prepare_gate: Option<Arc<Semaphore>>, fail: bool) -> Fixture {
    setup_tool(rounds, prepare_gate, fail, None)
}
fn setup_tool(
    rounds: usize,
    prepare_gate: Option<Arc<Semaphore>>,
    fail: bool,
    plan: Option<Arc<FileDelegationPlan>>,
) -> Fixture {
    let mut events = Vec::new();
    let mut streams = VecDeque::new();
    for _ in 0..rounds {
        let (tx, rx) = mpsc::unbounded_channel();
        events.push(tx);
        streams.push_back(rx);
    }
    let (starts, started) = mpsc::unbounded_channel();
    let (bindings, bound) = mpsc::unbounded_channel();
    let model = Arc::new(ModelFixture {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(streams),
        starts,
    });
    let resources = Arc::new(Resources {
        model,
        bindings,
        prepare_gate,
        fail: AtomicBool::new(fail),
        plan,
        claim_gate: Mutex::new(None),
    });
    let host = Host::memory(resources.clone()).unwrap();
    let connection = host.connect().unwrap();
    Fixture {
        resources,
        host,
        connection,
        events,
        starts: started,
        bindings: bound,
    }
}
async fn agent(f: &Fixture) -> SessionRef {
    let receipt = f
        .host
        .create_agent_async(
            &f.connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "delegation fixture".into(),
                mode: StorageMode::Memory,
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
fn input(mode: MessageMode) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode,
        text: "actual fixture task".into(),
        in_reply_to: None,
    }
}
fn send(f: &Fixture, target: &SessionRef) -> MessageInput {
    let input = input(MessageMode::FollowUp);
    f.host
        .send(
            &f.connection.caller(),
            SendMessage::new(input.clone(), target.clone()),
        )
        .unwrap();
    input
}
fn complete(events: &Events, text: &str) {
    events.send(ModelEvent::text_delta(text)).unwrap();
    events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        })
        .unwrap();
}
async fn recv<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("explicit gate must progress")
        .expect("sender alive")
}
async fn finished(f: &Fixture, target: &SessionRef, turn: &TurnId) {
    f.host
        .wait_turn(&f.connection.caller(), target, turn)
        .await
        .unwrap();
}
struct Hooks {
    sealed: AtomicBool,
    gate: Option<Arc<Semaphore>>,
    entered: Option<mpsc::UnboundedSender<()>>,
    closed: AtomicBool,
}
impl ScopeHooks for Hooks {
    fn seal(&self) {
        self.sealed.store(true, Ordering::SeqCst);
    }
    fn close_members(&self) -> OwnerFuture<'_, ()> {
        Box::pin(async {
            if let Some(entered) = &self.entered {
                entered.send(()).unwrap();
            }
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            Ok(())
        })
    }
    fn root_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}
fn hooks() -> Arc<Hooks> {
    Arc::new(Hooks {
        sealed: AtomicBool::new(false),
        gate: None,
        entered: None,
        closed: AtomicBool::new(false),
    })
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
    child: SessionRef,
    task: &MessageInput,
) -> DelegationRegistration {
    let s = cap.source();
    DelegationRegistration {
        host_id: s.host_id.clone(),
        host_incarnation: s.host_incarnation.clone(),
        root: s.session.clone(),
        parent: s.session.clone(),
        child,
        task_message_id: task.message_id.clone(),
        origin: ToolCallRef {
            turn_id: s.turn.turn_id.clone(),
            tool_call_id: ToolCallId("actual-bound-call".into()),
        },
        temporary: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn candidate_failure_never_publishes_turn_or_consumes_queued_input() {
    let gate = Arc::new(Semaphore::new(0));
    let mut f = setup(0, Some(gate.clone()), true);
    let target = agent(&f).await;
    let task = send(&f, &target);
    let candidate = recv(&mut f.bindings).await;
    assert!(candidate.capability().check_active().is_err());
    assert!(
        f.host
            .turns(&f.connection.caller(), &target)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &target, &task.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    let mut states = f
        .host
        .session_states(&f.connection.caller(), &target)
        .unwrap();
    gate.add_permits(1);
    while states.snapshot().unwrap().data.block_reason.is_none() {
        states.changed().await.unwrap();
    }
    assert!(candidate.capability().check_active().is_err());
    assert!(
        f.host
            .turns(&f.connection.caller(), &target)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_capability_outlives_external_connection_but_not_original_turn() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    let target = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    f.host.pause(&f.connection.caller(), &target).unwrap();
    f.connection.close();
    cap.check_active().unwrap();
    let message = input(MessageMode::FollowUp);
    cap.send(SendMessage::new(message.clone(), target.clone()))
        .unwrap();
    assert!(
        matches!(cap.message(&target, &message.message_id).unwrap().source, MessageSource::Agent { source } if source == parent)
    );
    let replacement = f.host.connect().unwrap();
    f.connection = replacement;
    complete(&f.events[0], "parent done");
    finished(&f, &parent, &cap.source().turn.turn_id).await;
    assert!(cap.check_active().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn busy_reuse_old_turn_cannot_finish_unconsumed_new_task() {
    let mut f = setup(3, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    send(&f, &child);
    let old = recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task.clone()).unwrap();
    complete(&f.events[1], "unrelated old final");
    let new = recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    assert_ne!(old.source().turn, new.source().turn);
    assert!(results.try_recv().is_err());
    complete(&f.events[2], "actual delegated final");
    let completion = recv(&mut results).await;
    assert_eq!(completion.results.len(), 1);
    assert_eq!(completion.results[0].info.id, new.source().turn.turn_id);
    assert_eq!(completion.inputs[0].message_ids, vec![task.message_id]);
    let notice = input(MessageMode::FollowUp);
    let receipt = permit.notify(notice).await.unwrap();
    assert_eq!(receipt.result.queue, InputQueue::NextStep);
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_handoff_survives_parent_cancel_and_notifies_paused_original_parent_once() {
    let mut f = setup(2, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task).unwrap();
    recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
    finished(&f, &parent, &cap.source().turn.turn_id).await;
    f.host.pause(&f.connection.caller(), &parent).unwrap();
    assert!(cap.check_active().is_err());
    complete(&f.events[1], "independent child result");
    recv(&mut results).await;
    let notice = input(MessageMode::Steer);
    let id = notice.message_id.clone();
    let receipt = permit.notify(notice).await.unwrap();
    assert_eq!(receipt.result.queue, InputQueue::NextTurn);
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &parent, &id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    assert!(permit.notify(input(MessageMode::Steer)).await.is_err());
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_ignores_next_turn_and_paused_next_step_without_consuming() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let waiting = cap.wait_input();
    tokio::pin!(waiting);
    assert!(waiting.as_mut().now_or_never().is_none());
    f.host
        .send(
            &f.connection.caller(),
            SendMessage::new(input(MessageMode::FollowUp), parent.clone()),
        )
        .unwrap();
    assert!(waiting.as_mut().now_or_never().is_none());
    f.host.pause(&f.connection.caller(), &parent).unwrap();
    let steer = input(MessageMode::Steer);
    f.host
        .send(
            &f.connection.caller(),
            SendMessage::new(steer.clone(), parent.clone()),
        )
        .unwrap();
    assert!(waiting.as_mut().now_or_never().is_none());
    f.host.resume(&f.connection.caller(), &parent).unwrap();
    waiting.await.unwrap();
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &parent, &steer.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_close_seals_receiving_and_waits_for_actual_cleanup_gate() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let (entered, mut entering) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let hooks = Arc::new(Hooks {
        sealed: AtomicBool::new(false),
        gate: Some(gate.clone()),
        entered: Some(entered),
        closed: AtomicBool::new(false),
    });
    let permit = cap.prepare_scope(hooks.clone()).unwrap();
    permit.seal().unwrap();
    assert!(
        f.host
            .send(
                &f.connection.caller(),
                SendMessage::new(input(MessageMode::FollowUp), parent.clone())
            )
            .is_err()
    );
    let cleanup = tokio::spawn(async move { permit.close_root(OperationId::new().unwrap()).await });
    recv(&mut entering).await;
    assert!(!cleanup.is_finished());
    assert!(!hooks.closed.load(Ordering::SeqCst));
    gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), cleanup)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(hooks.closed.load(Ordering::SeqCst));
}

struct FileDelegationPlan {
    path: std::path::PathBuf,
    tool_gate: Arc<Semaphore>,
    accepted: mpsc::UnboundedSender<(DelegationPermit, DelegationRegistration)>,
    sink: mpsc::UnboundedSender<DelegationCompletion>,
}
struct FileDelegatingTool {
    spec: minicore_runtime::tools::ToolSpec,
    cap: TurnCapability,
    plan: Arc<FileDelegationPlan>,
}
impl minicore_runtime::tools::Tool for FileDelegatingTool {
    fn spec(&self) -> &minicore_runtime::tools::ToolSpec {
        &self.spec
    }
    fn execute<'a>(
        &'a self,
        call: minicore_runtime::tools::ToolInvocation,
        _: minicore_runtime::tools::ToolContext,
    ) -> minicore_runtime::tools::ToolFuture<'a> {
        Box::pin(async move {
            let text = tokio::fs::read_to_string(&self.plan.path).await.unwrap();
            self.cap.prepare_scope(hooks()).unwrap();
            let (created, _) = self
                .cap
                .create_session_for_delegation(SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: AgentRef {
                        host_id: self.cap.source().host_id.clone(),
                        agent_id: self.cap.source().session.agent_id.clone(),
                    },
                    mode: Some(StorageMode::Memory),
                    overrides: Some(SessionOverrides(SessionConfigPatch {
                        tools: Some(Vec::new()),
                        ..Default::default()
                    })),
                    initial_message: None,
                })
                .await
                .unwrap();
            let child = SessionRef {
                agent_id: created.agent_id,
                session_id: created.session_id.unwrap(),
            };
            let mut task = input(MessageMode::FollowUp);
            task.text = text;
            let mut relation = registration(&self.cap, child, &task);
            relation.temporary = true;
            relation.origin.tool_call_id = ToolCallId(call.tool_call_id.as_str().into());
            let permit = self
                .cap
                .prepare_delegation(relation.clone(), Arc::new(Sink(self.plan.sink.clone())))
                .unwrap();
            self.cap.send_delegated(&relation, task).unwrap();
            self.cap.record_delegation(&relation.origin, &relation);
            self.plan.accepted.send((permit, relation)).unwrap();
            self.plan.tool_gate.acquire().await.unwrap().forget();
            Ok(minicore_runtime::tools::ToolExecutionOutcome::Completed(
                minicore_runtime::tools::ToolOutput::new("actual startup receipt").unwrap(),
            ))
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_file_tool_fast_child_hands_off_before_startup_tool_result_without_observers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("task.txt");
    std::fs::write(&path, "review this real file 🙂").unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let (accepted, mut receipts) = mpsc::unbounded_channel();
    let (sink, mut results) = mpsc::unbounded_channel();
    let mut f = setup_tool(
        3,
        None,
        false,
        Some(Arc::new(FileDelegationPlan {
            path,
            tool_gate: gate.clone(),
            accepted,
            sink,
        })),
    );
    let parent = agent(&f).await;
    f.host
        .update_session(
            &f.connection.caller(),
            &parent,
            0,
            &SessionConfigPatch {
                tools: Some(vec![ToolBinding {
                    name: "delegate_fixture".into(),
                }]),
                ..Default::default()
            },
        )
        .unwrap();
    send(&f, &parent);
    let parent_binding = recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    let id: minicore_runtime::ToolCallId = "reused-provider-id".parse().unwrap();
    f.events[0]
        .send(ModelEvent::ToolCallStart {
            tool_call_id: id.clone(),
            tool_name: "delegate_fixture".parse().unwrap(),
        })
        .unwrap();
    f.events[0]
        .send(ModelEvent::tool_call_arguments_delta(id.clone(), "{}"))
        .unwrap();
    f.events[0]
        .send(ModelEvent::ToolCallEnd { tool_call_id: id })
        .unwrap();
    f.events[0]
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        })
        .unwrap();
    let (permit, relation) = recv(&mut receipts).await;
    recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    complete(&f.events[1], "file reviewed successfully");
    let completion = recv(&mut results).await;
    assert_eq!(
        completion.inputs[0].message_ids,
        vec![relation.task_message_id.clone()]
    );
    assert!(
        f.host
            .toolcall_get(&f.connection.caller(), &parent, &relation.origin)
            .unwrap()
            .data
            .result
            .is_none()
    );
    permit.close(OperationId::new().unwrap()).await.unwrap();
    assert!(
        f.host
            .session_info(&f.connection.caller(), &relation.child)
            .is_err()
    );
    let receipt = permit.notify(input(MessageMode::FollowUp)).await.unwrap();
    assert_eq!(receipt.result.queue, InputQueue::NextStep);
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    gate.add_permits(1);
    recv(&mut f.starts).await;
    complete(&f.events[2], "parent summary");
    finished(&f, &parent, &parent_binding.source().turn.turn_id).await;
    let record = f
        .host
        .toolcall_get(&f.connection.caller(), &parent, &relation.origin)
        .unwrap();
    assert_eq!(record.data.state.delegations[0].child, relation.child);
    assert!(record.data.result.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn busy_steer_delegation_transfers_all_inputs_that_actually_shared_the_turn() {
    let mut f = setup(3, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let earlier = send(&f, &child);
    let child_binding = recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::Steer);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task.clone()).unwrap();
    let external = input(MessageMode::Steer);
    f.host
        .send(
            &f.connection.caller(),
            SendMessage::new(external.clone(), child.clone()),
        )
        .unwrap();
    complete(&f.events[1], "old input answered");
    recv(&mut f.starts).await;
    complete(&f.events[2], "joint output");
    let result = recv(&mut results).await;
    assert_eq!(result.inputs.len(), 1);
    assert_eq!(result.inputs[0].turn, child_binding.source().turn);
    assert_eq!(
        result.inputs[0].message_ids,
        vec![earlier.message_id, task.message_id, external.message_id]
    );
    let receipt = permit.notify(input(MessageMode::Steer)).await.unwrap();
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_but_unconsumed_task_cancellation_hands_off_discarded_fact() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    f.host.pause(&f.connection.caller(), &child).unwrap();
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task.clone()).unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &child, false)
        .unwrap();
    let completion = recv(&mut results).await;
    assert!(completion.results.is_empty());
    assert!(completion.inputs.is_empty());
    assert_eq!(completion.discarded_inputs, vec![task.message_id]);
    let receipt = permit.notify(input(MessageMode::Steer)).await.unwrap();
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_scope_close_wait_does_not_drop_actual_owner_cleanup() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let (entered, mut entering) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let hooks = Arc::new(Hooks {
        sealed: AtomicBool::new(false),
        gate: Some(gate.clone()),
        entered: Some(entered),
        closed: AtomicBool::new(false),
    });
    let permit = cap.prepare_scope(hooks.clone()).unwrap();
    let close = tokio::spawn(async move { permit.close_root(OperationId::new().unwrap()).await });
    recv(&mut entering).await;
    close.abort();
    let _ = close.await;
    // Register while cleanup still owns its gate. After release the memory
    // Session may already be reclaimed, which is successful cleanup.
    let mut states = f
        .host
        .session_states(&f.connection.caller(), &parent)
        .unwrap();
    gate.add_permits(1);
    loop {
        match states.snapshot() {
            Ok(snapshot) if snapshot.data.lifecycle != SessionLifecycle::Closed => {
                let _ = states.changed().await;
            }
            _ => break,
        }
    }
    assert!(hooks.sealed.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn cancellation_after_real_turn_binding_before_first_model_hands_off_cancelled_result() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let (entered, mut entering) = mpsc::unbounded_channel();
    let gate = Arc::new(std::sync::Barrier::new(2));
    *f.resources.claim_gate.lock().unwrap() = Some((entered, gate.clone()));
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task.clone()).unwrap();
    recv(&mut entering).await;
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &child, &task.message_id)
            .unwrap()
            .binding,
        MessageBinding::Bound { .. }
    ));
    f.host
        .cancel_session(&f.connection.caller(), &child, false)
        .unwrap();
    gate.wait();
    let completion = recv(&mut results).await;
    assert_eq!(completion.results.len(), 1);
    assert_eq!(
        completion.results[0].info.outcome,
        Some(TurnOutcome::Cancelled)
    );
    assert_eq!(completion.results[0].info.requests, 0);
    assert_eq!(completion.inputs[0].message_ids, vec![task.message_id]);
    let receipt = permit.notify(input(MessageMode::Steer)).await.unwrap();
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
}
struct GatedSink {
    entered: mpsc::UnboundedSender<()>,
    gate: Arc<std::sync::Barrier>,
    result: mpsc::UnboundedSender<DelegationCompletion>,
}
impl DelegationSink for GatedSink {
    fn handoff(&self, result: DelegationCompletion) -> Result<(), Fault> {
        self.entered.send(()).unwrap();
        self.gate.wait();
        self.result.send(result).unwrap();
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn memory_closed_cannot_publish_while_sink_transfer_is_still_running() {
    let mut f = setup(2, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (entered, mut entering) = mpsc::unbounded_channel();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(
            relation.clone(),
            Arc::new(GatedSink {
                entered,
                gate: gate.clone(),
                result: sink,
            }),
        )
        .unwrap();
    cap.send_delegated(&relation, task).unwrap();
    recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    complete(&f.events[1], "ready for transfer");
    recv(&mut entering).await;
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
    assert_eq!(
        f.host
            .session_state(&f.connection.caller(), &child)
            .unwrap()
            .data
            .lifecycle,
        SessionLifecycle::Closing
    );
    gate.wait();
    recv(&mut results).await;
    let receipt = permit.notify(input(MessageMode::Steer)).await.unwrap();
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_failure_cannot_finish_root_while_scope_cleanup_is_gated() {
    let preparation = Arc::new(Semaphore::new(1));
    let mut f = setup(1, Some(preparation.clone()), false);
    let parent = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let (entered, mut entering) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let hooks = Arc::new(Hooks {
        sealed: AtomicBool::new(false),
        gate: Some(gate.clone()),
        entered: Some(entered),
        closed: AtomicBool::new(false),
    });
    let permit = cap.prepare_scope(hooks).unwrap();
    complete(&f.events[0], "root first turn");
    finished(&f, &parent, &cap.source().turn.turn_id).await;
    f.resources.fail.store(true, Ordering::SeqCst);
    send(&f, &parent);
    recv(&mut f.bindings).await;
    let close = tokio::spawn(async move { permit.close_root(OperationId::new().unwrap()).await });
    recv(&mut entering).await;
    preparation.add_permits(1);
    let mut states = f
        .host
        .session_states(&f.connection.caller(), &parent)
        .unwrap();
    while states.snapshot().unwrap().data.preparing {
        states.changed().await.unwrap();
    }
    assert_eq!(
        states.snapshot().unwrap().data.lifecycle,
        SessionLifecycle::Closing
    );
    assert!(!close.is_finished());
    gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), close)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metadata_and_config_use_one_snapshot_revision_and_conflicts_do_not_overwrite() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let snapshot = cap.session_snapshot(&parent).unwrap();
    let mut config = snapshot.config.clone();
    config.role.instructions = "new effective role".into();
    let updated = cap
        .update_session(
            &parent,
            snapshot.config_revision,
            config.clone(),
            Some("new title".into()),
        )
        .await
        .unwrap();
    assert_eq!(updated.title.as_deref(), Some("new title"));
    assert_eq!(updated.config, config);
    assert_eq!(updated.config_revision, snapshot.config_revision + 1);
    assert_eq!(
        cap.update_session(
            &parent,
            snapshot.config_revision,
            snapshot.config,
            Some("stale title".into())
        )
        .await
        .unwrap_err()
        .kind(),
        Some(FaultKind::SessionConfigConflict)
    );
    assert_eq!(
        cap.session_snapshot(&parent).unwrap().title.as_deref(),
        Some("new title")
    );
    let target = AgentRef {
        host_id: cap.source().host_id.clone(),
        agent_id: parent.agent_id.clone(),
    };
    let snapshot = cap.agent_snapshot(&target).unwrap();
    let mut config = snapshot.config.clone();
    config.role.instructions = "new default role".into();
    let updated = cap
        .update_agent(
            &target,
            snapshot.config_revision,
            config.clone(),
            "new Agent name".into(),
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "new Agent name");
    assert_eq!(updated.config, config);
    assert_eq!(
        cap.update_agent(
            &target,
            snapshot.config_revision,
            snapshot.config,
            "stale Agent".into()
        )
        .await
        .unwrap_err()
        .kind(),
        Some(FaultKind::AgentConfigConflict)
    );
    assert_eq!(
        cap.session_snapshot(&parent)
            .unwrap()
            .config
            .role
            .instructions,
        "new effective role"
    );
    complete(&f.events[0], "bound old Turn still runs");
    finished(&f, &parent, &cap.source().turn.turn_id).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persisted_title_and_name_are_confirmed_with_the_same_config_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let mut f = setup(1, None, false);
    f.host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "metadata fixture".into(),
        },
        f.resources.clone(),
    )
    .await
    .unwrap();
    f.connection = f.host.connect().unwrap();
    let created = f
        .host
        .create_agent_async(
            &f.connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "before".into(),
                mode: StorageMode::Persistent,
                config: AgentConfig::default(),
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let parent = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let session = cap.session_snapshot(&parent).unwrap();
    let mut config = session.config;
    config.role.instructions = "persisted role".into();
    cap.update_session(
        &parent,
        session.config_revision,
        config.clone(),
        Some("persisted title".into()),
    )
    .await
    .unwrap();
    let session_path = directory
        .path()
        .join("agents")
        .join(parent.agent_id.as_str())
        .join("sessions")
        .join(parent.session_id.as_str())
        .join("session.json");
    let record: minicore_host_storage::SessionRecord =
        serde_json::from_slice(&std::fs::read(session_path).unwrap()).unwrap();
    assert_eq!(record.data.title.as_deref(), Some("persisted title"));
    assert_eq!(record.data.effective_config, config);
    assert_eq!(record.data.config_revision, 1);
    let target = AgentRef {
        host_id: cap.source().host_id.clone(),
        agent_id: parent.agent_id.clone(),
    };
    let agent = cap.agent_snapshot(&target).unwrap();
    cap.update_agent(
        &target,
        agent.config_revision,
        agent.config.clone(),
        "persisted name".into(),
    )
    .await
    .unwrap();
    let record: minicore_host_storage::AgentRecord = serde_json::from_slice(
        &std::fs::read(
            directory
                .path()
                .join("agents")
                .join(parent.agent_id.as_str())
                .join("agent.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record.data.name, "persisted name");
    assert_eq!(record.data.config, agent.config);
    assert_eq!(record.data.config_revision, 1);
    complete(&f.events[0], "done");
    finished(&f, &parent, &cap.source().turn.turn_id).await;
    let mut states = f
        .host
        .events_subscribe(
            &f.connection.caller(),
            SubscribeRequest {
                scope: SubscriptionScope::Host {
                    host_id: target.host_id.clone(),
                },
                categories: BTreeSet::from([EventCategory::State]),
            },
        )
        .unwrap();
    f.host
        .shutdown(
            &f.connection.caller(),
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
        )
        .unwrap();
    while f.host.info(&f.connection.caller()).unwrap().lifecycle != HostLifecycle::Stopped {
        states.recv().await.unwrap();
    }
    let reopened = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "ignored".into(),
        },
        f.resources.clone(),
    )
    .await
    .unwrap();
    let connection = reopened.connect().unwrap();
    assert_eq!(
        reopened
            .session_info(&connection.caller(), &parent)
            .unwrap()
            .title
            .as_deref(),
        Some("persisted title")
    );
    assert_eq!(
        reopened
            .agent_info(&connection.caller(), &target)
            .unwrap()
            .name,
        "persisted name"
    );
}

fn host_states(f: &Fixture) -> EventSubscription {
    let info = f.host.info(&f.connection.caller()).unwrap();
    f.host
        .events_subscribe(
            &f.connection.caller(),
            SubscribeRequest {
                scope: SubscriptionScope::Host {
                    host_id: info.host_id,
                },
                categories: BTreeSet::from([EventCategory::State]),
            },
        )
        .unwrap()
}
async fn await_stopped(f: &Fixture, mut events: EventSubscription) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while f.host.info(&f.connection.caller()).unwrap().lifecycle != HostLifecycle::Stopped {
            events.recv().await.unwrap();
        }
    })
    .await
    .expect("actual Host shutdown must finish");
}
async fn publish_partial(f: &Fixture, child: &SessionRef, events: &Events) {
    let mut output = f
        .host
        .events_subscribe(
            &f.connection.caller(),
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: child.agent_id.clone(),
                    session_id: child.session_id.clone(),
                },
                categories: BTreeSet::from([EventCategory::Output]),
            },
        )
        .unwrap();
    events
        .send(ModelEvent::text_delta("real partial response"))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(ObservationNotification::Event(push)) = output.recv().await.unwrap()
                && matches!(
                    push.event,
                    ObservationEvent::Activity(ActivityEvent::ModelOutput { .. })
                )
            {
                break;
            }
        }
    })
    .await
    .unwrap();
}
struct ClosingHooks {
    members: Mutex<Vec<DelegationPermit>>,
}
impl ScopeHooks for ClosingHooks {
    fn seal(&self) {}
    fn root_closed(&self) {}
    fn close_members(&self) -> OwnerFuture<'_, ()> {
        Box::pin(async {
            let members = std::mem::take(&mut *self.members.lock().unwrap());
            for member in members {
                member.close(OperationId::new().unwrap()).await?;
                member.release(DelegationDisposition::ScopeClosed).await?;
            }
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn draining_rejects_new_reused_session_delegation_but_accepts_registered_result() {
    let mut f = setup(2, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    let other = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task).unwrap();
    recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    let stopping = host_states(&f);
    f.host
        .shutdown(
            &f.connection.caller(),
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
        )
        .unwrap();
    let new_task = input(MessageMode::FollowUp);
    let new_relation = registration(&cap, other, &new_task);
    let (sink, _unused) = mpsc::unbounded_channel();
    assert_eq!(
        cap.prepare_delegation(new_relation, Arc::new(Sink(sink)))
            .err()
            .unwrap()
            .kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
    complete(&f.events[1], "existing work completes during draining");
    recv(&mut results).await;
    let receipt = permit.notify(input(MessageMode::FollowUp)).await.unwrap();
    assert_eq!(receipt.result.queue, InputQueue::NextStep);
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
    await_stopped(&f, stopping).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn force_joins_registered_partial_handoff_and_cleanup_without_new_parent_turn() {
    let mut f = setup(2, None, false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let hooks = Arc::new(ClosingHooks {
        members: Mutex::new(Vec::new()),
    });
    cap.prepare_scope(hooks.clone()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    hooks.members.lock().unwrap().push(permit.clone());
    cap.send_delegated(&relation, task).unwrap();
    recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    publish_partial(&f, &child, &f.events[1]).await;
    let stopping = host_states(&f);
    f.host
        .shutdown(
            &f.connection.caller(),
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Force,
            },
        )
        .unwrap();
    let result = recv(&mut results).await;
    assert_eq!(result.results[0].info.outcome, Some(TurnOutcome::Cancelled));
    assert!(!result.results[0].partials.is_empty());
    assert!(permit.notify(input(MessageMode::Steer)).await.is_err());
    await_stopped(&f, stopping).await;
    assert!(f.starts.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn persistent_partial_cancel_with_preserved_inbox_waits_for_actual_next_turn() {
    let directory = tempfile::tempdir().unwrap();
    let mut f = setup(3, None, false);
    f.host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "partial save fixture".into(),
        },
        f.resources.clone(),
    )
    .await
    .unwrap();
    f.connection = f.host.connect().unwrap();
    let parent = agent(&f).await;
    let created = f
        .host
        .create_agent_async(
            &f.connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "persistent child".into(),
                mode: StorageMode::Persistent,
                config: AgentConfig::default(),
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let child = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task).unwrap();
    let first = recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    publish_partial(&f, &child, &f.events[1]).await;
    let followup = input(MessageMode::FollowUp);
    cap.send_delegated(&relation, followup.clone()).unwrap();
    f.host.pause(&f.connection.caller(), &child).unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &child, true)
        .unwrap();
    finished(&f, &child, &first.source().turn.turn_id).await;
    assert!(results.try_recv().is_err());
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &child, &followup.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    f.host.resume(&f.connection.caller(), &child).unwrap();
    recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    complete(&f.events[2], "remaining input actually completed");
    let result = recv(&mut results).await;
    assert_eq!(result.results.len(), 2);
    assert_eq!(result.results[0].info.outcome, Some(TurnOutcome::Cancelled));
    assert!(!result.results[0].partials.is_empty());
    assert!(
        result
            .results
            .iter()
            .all(|turn| turn.info.persistence == PersistenceState::Persisted)
    );
    assert_eq!(result.results[1].info.outcome, Some(TurnOutcome::Completed));
    let path = directory
        .path()
        .join("agents")
        .join(child.agent_id.as_str())
        .join("sessions")
        .join(child.session_id.as_str())
        .join("history.jsonl");
    let records: Vec<minicore_host_storage::HistoryRecord> = std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records.iter().any(|record| matches!(
        record.entry,
        minicore_host_storage::HistoryEntry::Partial { .. }
    )));
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(
                record.entry,
                minicore_host_storage::HistoryEntry::TurnFinished { .. }
            ))
            .count(),
        2
    );
    let receipt = permit.notify(input(MessageMode::Steer)).await.unwrap();
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
    let stopping = host_states(&f);
    f.host
        .shutdown(
            &f.connection.caller(),
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
        )
        .unwrap();
    await_stopped(&f, stopping).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn grandchild_keeps_running_after_direct_parent_release_and_never_retargets_root() {
    let mut f = setup(3, None, false);
    let root = agent(&f).await;
    let middle = agent(&f).await;
    let grandchild = agent(&f).await;
    send(&f, &root);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let middle_task = input(MessageMode::FollowUp);
    let middle_relation = registration(&cap, middle.clone(), &middle_task);
    let (sink, mut middle_results) = mpsc::unbounded_channel();
    let middle_permit = cap
        .prepare_delegation(middle_relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&middle_relation, middle_task).unwrap();
    let middle_cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let task = input(MessageMode::FollowUp);
    let mut relation = registration(&middle_cap, grandchild.clone(), &task);
    relation.root = root.clone();
    let (sink, mut grand_results) = mpsc::unbounded_channel();
    let grand_permit = middle_cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    middle_cap.send_delegated(&relation, task).unwrap();
    recv(&mut f.bindings).await;
    recv(&mut f.starts).await;
    middle_permit
        .close(OperationId::new().unwrap())
        .await
        .unwrap();
    recv(&mut middle_results).await;
    let receipt = middle_permit
        .notify(input(MessageMode::Steer))
        .await
        .unwrap();
    middle_permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    assert!(
        f.host
            .session_info(&f.connection.caller(), &middle)
            .is_err()
    );
    let queued_before = f
        .host
        .session_state(&f.connection.caller(), &root)
        .unwrap()
        .data
        .next_step
        .len();
    complete(&f.events[2], "grandchild finishes independently");
    recv(&mut grand_results).await;
    let fault = grand_permit
        .notify(input(MessageMode::Steer))
        .await
        .unwrap_err();
    assert!(matches!(
        fault.kind(),
        Some(FaultKind::AgentSessionNotFound | FaultKind::HostLifecycleUnavailable)
    ));
    assert_eq!(
        f.host
            .session_state(&f.connection.caller(), &root)
            .unwrap()
            .data
            .next_step
            .len(),
        queued_before
    );
    grand_permit
        .release(DelegationDisposition::NotificationFailed(fault))
        .await
        .unwrap();
    f.host
        .cancel_session(&f.connection.caller(), &root, false)
        .unwrap();
}
struct FailOnceHooks {
    attempts: std::sync::atomic::AtomicUsize,
    entered: mpsc::UnboundedSender<()>,
}
impl ScopeHooks for FailOnceHooks {
    fn seal(&self) {}
    fn root_closed(&self) {}
    fn close_members(&self) -> OwnerFuture<'_, ()> {
        Box::pin(async {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            self.entered.send(()).unwrap();
            if attempt == 0 {
                Err(Fault::new(
                    FaultKind::SessionHistoryWriteFailed,
                    "explicit cleanup failure fixture",
                ))
            } else {
                Ok(())
            }
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_cleanup_failure_only_continues_on_explicit_resume_without_business_restart() {
    let mut f = setup(1, None, false);
    let parent = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    let (entered, mut entering) = mpsc::unbounded_channel();
    let hooks = Arc::new(FailOnceHooks {
        attempts: std::sync::atomic::AtomicUsize::new(0),
        entered,
    });
    cap.prepare_scope(hooks.clone()).unwrap();
    let stopping = host_states(&f);
    f.host
        .shutdown(
            &f.connection.caller(),
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Force,
            },
        )
        .unwrap();
    recv(&mut entering).await;
    let mut states = f
        .host
        .session_states(&f.connection.caller(), &parent)
        .unwrap();
    loop {
        let state = states.snapshot().unwrap().data;
        if state.metadata_fault.is_some() && state.active_turn_id.is_none() && !state.saving {
            break;
        }
        states.changed().await.unwrap();
    }
    assert_eq!(hooks.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.host.info(&f.connection.caller()).unwrap().lifecycle,
        HostLifecycle::Stopping
    );
    f.host.resume(&f.connection.caller(), &parent).unwrap();
    recv(&mut entering).await;
    await_stopped(&f, stopping).await;
    assert_eq!(hooks.attempts.load(Ordering::SeqCst), 2);
    assert!(f.starts.try_recv().is_err());
}

async fn close_during_preparation_hands_off_discarded_task(fail: bool) {
    let preparation = Arc::new(Semaphore::new(1));
    let mut f = setup(1, Some(preparation.clone()), false);
    let parent = agent(&f).await;
    let child = agent(&f).await;
    send(&f, &parent);
    let cap = recv(&mut f.bindings).await.capability();
    recv(&mut f.starts).await;
    cap.prepare_scope(hooks()).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, child.clone(), &task);
    let (sink, mut results) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task.clone()).unwrap();
    let candidate = recv(&mut f.bindings).await;
    assert!(candidate.capability().check_active().is_err());
    f.resources.fail.store(fail, Ordering::SeqCst);
    let mut states = f
        .host
        .session_states(&f.connection.caller(), &child)
        .unwrap();
    let closing = permit.clone();
    let close = tokio::spawn(async move { closing.close(OperationId::new().unwrap()).await });
    while states.snapshot().unwrap().data.lifecycle != SessionLifecycle::Closing {
        states.changed().await.unwrap();
    }
    assert!(results.try_recv().is_err());
    assert!(!close.is_finished());
    preparation.add_permits(1);
    let completion = recv(&mut results).await;
    assert!(completion.results.is_empty());
    assert!(completion.inputs.is_empty());
    assert_eq!(completion.discarded_inputs, vec![task.message_id]);
    tokio::time::timeout(Duration::from_secs(10), close)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let receipt = permit.notify(input(MessageMode::FollowUp)).await.unwrap();
    permit
        .release(DelegationDisposition::Notified(receipt))
        .await
        .unwrap();
    assert!(f.starts.try_recv().is_err());
    f.host
        .cancel_session(&f.connection.caller(), &parent, false)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_preparation_after_close_transfers_discarded_task_before_retirement() {
    close_during_preparation_hands_off_discarded_task(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_preparation_after_close_transfers_discarded_task_before_retirement() {
    close_during_preparation_hands_off_discarded_task(true).await;
}
