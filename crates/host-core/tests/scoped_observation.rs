//! Real owners and AgentLoop, with explicitly scripted/gated model and file tool.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
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

type Events = mpsc::UnboundedSender<ModelEvent>;
struct ScriptedModel {
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
struct Resources {
    model: Arc<dyn Model>,
    tool: Option<Arc<dyn minicore_runtime::tools::Tool>>,
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
                options: LoopOptions::default_checked().unwrap(),
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
fn setup(
    rounds: usize,
    tool: Option<Arc<dyn minicore_runtime::tools::Tool>>,
) -> (
    Arc<Host>,
    ConnectionHandle,
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
    let host = Host::memory(Arc::new(Resources { model, tool })).unwrap();
    let connection = host.connect().unwrap();
    (host, connection, inputs, starts)
}
fn agent(host: &Host, caller: &CallerContext) -> SessionRef {
    let receipt = host
        .create_agent(
            caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "observation fixture".into(),
                mode: StorageMode::Memory,
                config: AgentConfig::default(),
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
fn send(host: &Host, caller: &CallerContext, target: &SessionRef, mode: MessageMode) {
    host.send(
        caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode,
                text: "observe".into(),
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
}
fn subscribe(
    host: &Host,
    caller: &CallerContext,
    scope: SubscriptionScope,
    categories: &[EventCategory],
) -> EventSubscription {
    host.events_subscribe(
        caller,
        SubscribeRequest {
            scope,
            categories: categories.iter().copied().collect(),
        },
    )
    .unwrap()
}
fn session_scope(target: &SessionRef) -> SubscriptionScope {
    SubscriptionScope::Session {
        agent_id: target.agent_id.clone(),
        session_id: target.session_id.clone(),
    }
}
async fn next(subscription: &mut EventSubscription) -> ObservationEvent {
    match tokio::time::timeout(std::time::Duration::from_secs(10), subscription.recv())
        .await
        .expect("gated event timeout")
        .unwrap()
        .expect("live subscription")
    {
        ObservationNotification::Event(value) => value.event,
        other => panic!("unexpected control notification: {other:?}"),
    }
}
async fn delta(subscription: &mut EventSubscription) -> (ModelOutputIdentity, OutputDelta) {
    loop {
        if let ObservationEvent::Activity(ActivityEvent::ModelOutput { source, data }) =
            next(subscription).await
        {
            return (source, data);
        }
    }
}
async fn finish(subscription: &mut EventSubscription) -> TurnIdentity {
    loop {
        if let ObservationEvent::Activity(ActivityEvent::TurnFinished { source, .. }) =
            next(subscription).await
        {
            return source;
        }
    }
}
fn read(start: u64, end: u64, max_bytes: u64) -> TextReadRequest {
    TextReadRequest {
        channel: "text".into(),
        start,
        end,
        max_bytes,
    }
}
fn complete(events: &Events, text: &str) {
    events.send(ModelEvent::text_delta(text)).unwrap();
    events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        })
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_watermark_and_fixed_utf8_reads_stitch_real_output() {
    let (host, connection, events, mut starts) = setup(1, None);
    let caller = connection.caller();
    let target = agent(&host, &caller);
    let mut sub = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[
            EventCategory::State,
            EventCategory::Output,
            EventCategory::Completion,
            EventCategory::Message,
        ],
    );
    assert!(
        sub.result()
            .initial_states
            .iter()
            .any(|s| matches!(s, StateEvent::Session { .. }))
    );
    send(&host, &caller, &target, MessageMode::FollowUp);
    starts.recv().await.unwrap();
    events[0].send(ModelEvent::text_delta("中🙂a")).unwrap();
    let (source, chunk) = delta(&mut sub).await;
    assert_eq!((chunk.start, chunk.end), (0, 8));
    assert_eq!(source.response_index, 0);
    let baseline = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::Output],
    );
    assert!(baseline.result().initial_states.is_empty());
    assert!(
        baseline
            .result()
            .output_positions
            .iter()
            .any(|p| p.channel == "text" && p.end == 8)
    );
    let first = host
        .turn_output_read(&caller, &target, &source.turn.turn_id, 0, &read(0, 8, 1))
        .unwrap();
    assert_eq!(first.data.text, "中");
    assert_eq!(first.data.end, 3);
    assert!(first.data.has_more);
    events[0].send(ModelEvent::text_delta("尾")).unwrap();
    delta(&mut sub).await;
    let fixed = host
        .turn_output_read(&caller, &target, &source.turn.turn_id, 0, &read(3, 8, 100))
        .unwrap();
    assert_eq!(fixed.data.text, "🙂a");
    assert!(!fixed.data.has_more);
    assert_eq!(fixed.data.end, 8);
    assert_eq!(
        host.turn_output_read(&caller, &target, &source.turn.turn_id, 0, &read(1, 8, 2))
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionOutputRangeInvalid)
    );
    events[0]
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        })
        .unwrap();
    let ended = finish(&mut sub).await;
    assert_eq!(ended.turn_id, source.turn.turn_id);
    let tail = host
        .turn_output_read(&caller, &target, &source.turn.turn_id, 0, &read(8, 11, 1))
        .unwrap();
    assert_eq!(tail.data.text, "尾");
    let empty = host
        .turn_output_read(&caller, &target, &source.turn.turn_id, 0, &read(11, 11, 1))
        .unwrap();
    assert_eq!(empty.data.text, "");
    assert!(!empty.data.has_more);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scopes_cover_new_direct_descendants_and_do_not_expand_to_siblings() {
    let (host, connection, events, mut starts) = setup(2, None);
    let caller = connection.caller();
    let mut all = subscribe(
        &host,
        &caller,
        SubscriptionScope::Host {
            host_id: host.info(&caller).unwrap().host_id,
        },
        &[EventCategory::Output],
    );
    let first = agent(&host, &caller);
    let second = agent(&host, &caller);
    let mut own = subscribe(
        &host,
        &caller,
        session_scope(&first),
        &[EventCategory::Output],
    );
    send(&host, &caller, &second, MessageMode::FollowUp);
    starts.recv().await.unwrap();
    complete(&events[0], "second");
    let (outside, _) = delta(&mut all).await;
    assert_eq!(outside.turn.session.session_id, second.session_id);
    send(&host, &caller, &first, MessageMode::FollowUp);
    starts.recv().await.unwrap();
    complete(&events[1], "first");
    let (inside, _) = delta(&mut own).await;
    assert_eq!(inside.turn.session.session_id, first.session_id);
    let (global, _) = delta(&mut all).await;
    assert_eq!(global.turn.session.session_id, first.session_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscription_cleanup_is_connection_bound_and_old_ids_do_not_end_new_scopes() {
    let (host, connection, events, mut starts) = setup(1, None);
    let caller = connection.caller();
    let target = agent(&host, &caller);
    let read_only = host.connect().unwrap();
    let reader = read_only.caller();
    let mut old = subscribe(
        &host,
        &reader,
        session_scope(&target),
        &[EventCategory::Output],
    );
    let old_id = old.result().subscription_id.clone();
    assert_eq!(
        host.events_unsubscribe(
            &caller,
            &UnsubscribeRequest {
                subscription_id: old_id.clone()
            }
        )
        .unwrap_err()
        .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    old.end_after_delivery_failure(Fault::new(
        FaultKind::HostObservationResyncRequired,
        "test adapter recorded a real dispatch failure",
    ));
    assert!(matches!(
        old.recv().await.unwrap(),
        Some(ObservationNotification::ResyncRequired(_))
    ));
    assert!(old.recv().await.unwrap().is_none());
    let mut fresh = subscribe(
        &host,
        &reader,
        session_scope(&target),
        &[EventCategory::Output],
    );
    assert_ne!(fresh.result().subscription_id, old_id);
    assert!(
        !host
            .events_unsubscribe(
                &reader,
                &UnsubscribeRequest {
                    subscription_id: old_id
                }
            )
            .unwrap()
            .ended
    );
    drop(old);
    send(&host, &caller, &target, MessageMode::FollowUp);
    starts.recv().await.unwrap();
    events[0].send(ModelEvent::text_delta("live")).unwrap();
    delta(&mut fresh).await;
    read_only.close();
    assert_eq!(
        fresh.recv().await.unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    complete(&events[0], " continues");
    let mut states = host.session_states(&caller, &target).unwrap();
    loop {
        if states.snapshot().unwrap().data.last_result.is_some() {
            break;
        }
        states.changed().await.unwrap();
    }
    let turn = host.turns(&caller, &target).unwrap().pop().unwrap();
    assert_eq!(
        host.turn_output_read(&caller, &target, &turn.id, 0, &read(0, 14, 100))
            .unwrap()
            .data
            .text,
        "live continues"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn response_indices_continue_across_internal_loops_in_one_turn() {
    let (host, connection, events, mut starts) = setup(2, None);
    let caller = connection.caller();
    let target = agent(&host, &caller);
    let mut sub = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::Output, EventCategory::Completion],
    );
    send(&host, &caller, &target, MessageMode::FollowUp);
    starts.recv().await.unwrap();
    send(&host, &caller, &target, MessageMode::Steer);
    complete(&events[0], "first");
    let (first, _) = delta(&mut sub).await;
    starts.recv().await.unwrap();
    complete(&events[1], "second");
    let (second, _) = delta(&mut sub).await;
    assert_eq!(first.turn.turn_id, second.turn.turn_id);
    assert_eq!((first.response_index, second.response_index), (0, 1));
    finish(&mut sub).await;
    assert_eq!(
        host.turn_output_read(&caller, &target, &first.turn.turn_id, 0, &read(0, 5, 100))
            .unwrap()
            .data
            .text,
        "first"
    );
    assert_eq!(
        host.turn_output_read(&caller, &target, &first.turn.turn_id, 1, &read(0, 6, 100))
            .unwrap()
            .data
            .text,
        "second"
    );
}

struct GatedFileTool {
    spec: minicore_runtime::tools::ToolSpec,
    entered: mpsc::UnboundedSender<()>,
    gate: Arc<tokio::sync::Semaphore>,
    retained_sink: Arc<Mutex<Option<ToolOutputSink>>>,
}
impl minicore_runtime::tools::Tool for GatedFileTool {
    fn spec(&self) -> &minicore_runtime::tools::ToolSpec {
        &self.spec
    }
    fn execute<'a>(
        &'a self,
        invocation: minicore_runtime::tools::ToolInvocation,
        context: minicore_runtime::tools::ToolContext,
    ) -> minicore_runtime::tools::ToolFuture<'a> {
        Box::pin(async move {
            let content = tokio::fs::read_to_string(invocation.arguments["path"].as_str().unwrap())
                .await
                .unwrap();
            let sink = ToolOutputSink::current().expect("trusted invocation binding");
            assert!(sink.emit("stdout", &content));
            *self.retained_sink.lock().unwrap() = Some(sink);
            context.progress.emit(
                minicore_runtime::tools::ToolProgress::new(
                    Some(minicore_runtime::Text::new("file read")),
                    Some(1),
                    Some(1),
                )
                .unwrap(),
            );
            self.entered.send(()).unwrap();
            self.gate.acquire().await.unwrap().forget();
            Ok(minicore_runtime::tools::ToolExecutionOutcome::Completed(
                minicore_runtime::tools::ToolOutput::new(format!("result: {content}")).unwrap(),
            ))
        })
    }
}
fn call_file(events: &Events, path: &std::path::Path) {
    let id: minicore_runtime::ToolCallId = "repeated-call".parse().unwrap();
    events
        .send(ModelEvent::ToolCallStart {
            tool_call_id: id.clone(),
            tool_name: "read_fixture".parse().unwrap(),
        })
        .unwrap();
    events
        .send(ModelEvent::tool_call_arguments_delta(
            id.clone(),
            serde_json::json!({"path":path}).to_string(),
        ))
        .unwrap();
    events
        .send(ModelEvent::ToolCallEnd { tool_call_id: id })
        .unwrap();
    events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        })
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_tool_process_output_and_complete_results_have_separate_owners() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("actual.txt");
    std::fs::write(&path, "读🙂").unwrap();
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let retained_sink = Arc::new(Mutex::new(None));
    let tool = Arc::new(GatedFileTool {
        spec: minicore_runtime::tools::ToolSpec::new(
            "read_fixture".parse().unwrap(),
            "read a real fixture file",
            serde_json::json!({"type":"object"}),
        )
        .unwrap(),
        entered,
        gate: Arc::clone(&gate),
        retained_sink: Arc::clone(&retained_sink),
    });
    let (host, connection, events, mut starts) = setup(4, Some(tool));
    let caller = connection.caller();
    let target = agent(&host, &caller);
    let mut sub = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[
            EventCategory::State,
            EventCategory::Output,
            EventCategory::Completion,
        ],
    );
    let mut turns = Vec::new();
    for round in 0..2 {
        send(&host, &caller, &target, MessageMode::FollowUp);
        starts.recv().await.unwrap();
        call_file(&events[round * 2], &path);
        arrivals.recv().await.unwrap();
        let (source, data) = loop {
            if let ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) =
                next(&mut sub).await
            {
                break (source, data);
            }
        };
        assert_eq!(data.text, "读🙂");
        assert_eq!((data.start, data.end), (0, 7));
        let tool_ref = ToolCallRef {
            turn_id: source.turn.turn_id.clone(),
            tool_call_id: source.tool_call_id.clone(),
        };
        let current = host.toolcall_get(&caller, &target, &tool_ref).unwrap();
        assert_eq!(current.data.arguments, serde_json::json!({"path":path}));
        assert!(current.data.result.is_none());
        let before = host
            .toolcall_output_read(
                &caller,
                &target,
                &tool_ref,
                &TextReadRequest {
                    channel: "stdout".into(),
                    start: 0,
                    end: 7,
                    max_bytes: 1,
                },
            )
            .unwrap();
        assert_eq!(before.data.text, "读");
        assert!(before.data.has_more);
        gate.add_permits(1);
        let done = loop {
            if let ObservationEvent::Activity(ActivityEvent::ToolFinished {
                source: ended,
                data,
            }) = next(&mut sub).await
            {
                assert_eq!(ended, source);
                break data;
            }
        };
        assert_eq!(done.result.output.content().as_str(), "result: 读🙂");
        assert_eq!(
            done.output_positions,
            vec![ChannelPosition {
                channel: "stdout".into(),
                end: 7
            }]
        );
        assert!(
            !retained_sink
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .emit("stdout", "late")
        );
        starts.recv().await.unwrap();
        complete(&events[round * 2 + 1], "done");
        finish(&mut sub).await;
        assert_eq!(
            host.toolcall_output_read(
                &caller,
                &target,
                &tool_ref,
                &TextReadRequest {
                    channel: "stdout".into(),
                    start: 0,
                    end: 7,
                    max_bytes: 100
                }
            )
            .unwrap()
            .data
            .text,
            "读🙂"
        );
        turns.push(tool_ref);
    }
    assert_ne!(turns[0].turn_id, turns[1].turn_id);
    assert_eq!(turns[0].tool_call_id, turns[1].tool_call_id);
    assert!(
        host.toolcall_get(&caller, &target, &turns[0])
            .unwrap()
            .data
            .result
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_subscription_does_not_retain_reclaimed_source() {
    let (host, connection, events, mut starts) = setup(1, None);
    let caller = connection.caller();
    let target = agent(&host, &caller);
    send(&host, &caller, &target, MessageMode::FollowUp);
    starts.recv().await.unwrap();
    complete(&events[0], "retained by owner");
    let mut state = host.session_states(&caller, &target).unwrap();
    loop {
        if state.snapshot().unwrap().data.last_result.is_some() {
            break;
        }
        state.changed().await.unwrap();
    }
    let turn = host.turns(&caller, &target).unwrap().pop().unwrap();
    let retained = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::Output],
    );
    assert!(
        retained
            .result()
            .output_positions
            .iter()
            .any(|p| p.end == 17)
    );
    assert_eq!(
        host.turn_output_read(&caller, &target, &turn.id, 0, &read(0, 17, 100))
            .unwrap()
            .data
            .text,
        "retained by owner"
    );
    host.close_session(&caller, &target).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match state.snapshot() {
                Err(_) => break,
                Ok(_) => {
                    if state.changed().await.is_err() {
                        break;
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(
        host.turn_output_read(&caller, &target, &turn.id, 0, &read(0, 17, 100))
            .is_err()
    );
    drop(retained);
}
