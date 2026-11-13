//! Real Session owners and internal model/tool primitives, with scripted/gated fixtures.
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
    tool: Option<Arc<dyn minicore_host_runtime::tools::Tool>>,
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
        // Probe the trusted output boundary with altered invocations before
        // forwarding the real Runtime call to the file-reading fixture.
        tools.register_arc(Arc::new(InvocationIdentityProbe {
            inner: output.wrap(Arc::clone(tool), tool.spec().clone()),
        }));
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
    tool: Option<Arc<dyn minicore_host_runtime::tools::Tool>>,
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
async fn agent(host: &Arc<Host>, caller: &CallerContext) -> SessionRef {
    let receipt = host
        .create_agent_async(
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
        .await
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
    let target = agent(&host, &caller).await;
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
    let first = agent(&host, &caller).await;
    let second = agent(&host, &caller).await;
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
    let target = agent(&host, &caller).await;
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
    let target = agent(&host, &caller).await;
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

struct InvocationIdentityProbe {
    inner: Arc<dyn minicore_host_runtime::tools::Tool>,
}
impl minicore_host_runtime::tools::Tool for InvocationIdentityProbe {
    fn spec(&self) -> &minicore_host_runtime::tools::ToolSpec {
        self.inner.spec()
    }
    fn execute<'a>(
        &'a self,
        invocation: minicore_host_runtime::tools::ToolInvocation,
        context: minicore_host_runtime::tools::ToolContext,
    ) -> minicore_host_runtime::tools::ToolFuture<'a> {
        Box::pin(async move {
            for field in ["call", "name", "arguments"] {
                let mut altered = invocation.clone();
                match field {
                    "call" => altered.tool_call_id = "unregistered-call".parse().unwrap(),
                    "name" => altered.tool_name = "unregistered_tool".parse().unwrap(),
                    "arguments" => altered.arguments["unexpected"] = serde_json::json!(true),
                    _ => unreachable!(),
                }
                assert!(matches!(
                    self.inner.execute(altered, context.clone()).await,
                    Err(minicore_host_runtime::tools::ToolError::Internal)
                ));
            }
            self.inner.execute(invocation, context).await
        })
    }
}
struct GatedFileTool {
    spec: minicore_host_runtime::tools::ToolSpec,
    entered: mpsc::UnboundedSender<()>,
    gate: Arc<tokio::sync::Semaphore>,
    retained_sink: Arc<Mutex<Option<ToolOutputSink>>>,
}
impl minicore_host_runtime::tools::Tool for GatedFileTool {
    fn spec(&self) -> &minicore_host_runtime::tools::ToolSpec {
        &self.spec
    }
    fn execute<'a>(
        &'a self,
        invocation: minicore_host_runtime::tools::ToolInvocation,
        context: minicore_host_runtime::tools::ToolContext,
    ) -> minicore_host_runtime::tools::ToolFuture<'a> {
        Box::pin(async move {
            let content = tokio::fs::read_to_string(invocation.arguments["path"].as_str().unwrap())
                .await
                .unwrap();
            let sink = ToolOutputSink::current().expect("trusted invocation binding");
            assert!(sink.emit("stdout", &content));
            *self.retained_sink.lock().unwrap() = Some(sink);
            context.progress.emit(
                minicore_host_runtime::tools::ToolProgress::new(
                    Some(minicore_host_runtime::Text::new("file read")),
                    Some(1),
                    Some(1),
                )
                .unwrap(),
            );
            self.entered.send(()).unwrap();
            self.gate.acquire().await.unwrap().forget();
            Ok(
                minicore_host_runtime::tools::ToolExecutionOutcome::Completed(
                    minicore_host_runtime::tools::ToolOutput::new(format!("result: {content}"))
                        .unwrap(),
                ),
            )
        })
    }
}
fn call_file(events: &Events, path: &std::path::Path) {
    let id: minicore_host_runtime::ToolCallId = "repeated-call".parse().unwrap();
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
        spec: minicore_host_runtime::tools::ToolSpec::new(
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
    let target = agent(&host, &caller).await;
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
    let mut previous_sink: Option<ToolOutputSink> = None;
    for round in 0..2 {
        send(&host, &caller, &target, MessageMode::FollowUp);
        starts.recv().await.unwrap();
        let active = host
            .session_state(&caller, &target)
            .unwrap()
            .data
            .active_turn_id
            .unwrap();
        let mut turn_scope = subscribe(
            &host,
            &caller,
            SubscriptionScope::Turn {
                session_id: target.session_id.clone(),
                turn_id: active.clone(),
            },
            &[EventCategory::Output],
        );
        call_file(&events[round * 2], &path);
        arrivals.recv().await.unwrap();
        if let Some(previous) = &previous_sink {
            // The same ToolCallId in a later Turn cannot revive an old sink.
            assert!(!previous.emit("stdout", "wrong Turn"));
            assert!(previous.retain_owned().is_err());
        }
        previous_sink = retained_sink.lock().unwrap().clone();
        let (source, data) = loop {
            if let ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) =
                next(&mut sub).await
            {
                break (source, data);
            }
        };
        loop {
            if let ObservationEvent::Activity(ActivityEvent::ToolOutput {
                source: scoped, ..
            }) = next(&mut turn_scope).await
            {
                assert_eq!(scoped.turn.turn_id, active);
                break;
            }
        }
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
    let target = agent(&host, &caller).await;
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

#[tokio::test]
async fn finished_snapshot_and_delayed_output_reader_keep_original_source_bytes() {
    let (host, connection, events, _starts) = setup(1, None);
    let caller = connection.caller();
    let target = agent(&host, &caller).await;
    let mut sub = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::Output, EventCategory::Completion],
    );
    complete(&events[0], "early complete🙂");
    send(&host, &caller, &target, MessageMode::FollowUp);
    // A slow subscriber can read its queued deltas after the authoritative
    // Turn snapshot has finished. Delivery does not own completion.
    let mut states = host.session_states(&caller, &target).unwrap();
    while states.snapshot().unwrap().data.last_result.is_none() {
        states.changed().await.unwrap();
    }
    let (tail, delta) = delta(&mut sub).await;
    let (finished, positions) = match next(&mut sub).await {
        ObservationEvent::Activity(ActivityEvent::ModelFinished { source, data }) => {
            (source, data.output_positions)
        }
        other => panic!("expected the actual complete response: {other:?}"),
    };
    assert_eq!(tail, finished);
    assert_eq!(delta.text, "early complete🙂");
    assert_eq!(
        positions.iter().find(|p| p.channel == "text").unwrap().end,
        delta.end
    );
    assert_eq!(
        host.turn_output_read(
            &caller,
            &target,
            &tail.turn.turn_id,
            0,
            &read(0, delta.end, 100)
        )
        .unwrap()
        .data
        .text,
        delta.text
    );
    finish(&mut sub).await;
}

struct RetryModel {
    descriptor: ModelDescriptor,
    calls: std::sync::atomic::AtomicUsize,
    starts: mpsc::UnboundedSender<usize>,
    release: Arc<tokio::sync::Semaphore>,
}
impl Model for RetryModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let attempt = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.starts.send(attempt).unwrap();
        Box::pin(async move {
            if attempt == 1 {
                return Err(minicore_host_runtime::model::ModelError::not_started(
                    minicore_host_runtime::model::ModelErrorKind::ProviderUnavailable,
                    None,
                    minicore_host_runtime::error::DiagnosticSummary::new(
                        minicore_host_runtime::error::DiagnosticCode::ModelUnavailable,
                        minicore_host_runtime::error::DiagnosticCategory::Model,
                        minicore_host_runtime::Text::new("retry fixture"),
                        true,
                    )
                    .unwrap(),
                ));
            }
            self.release.acquire().await.unwrap().forget();
            Ok(Box::pin(stream::iter([
                Ok(ModelEvent::text_delta("recovered")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
struct RetryResources {
    model: Arc<RetryModel>,
    delay: std::time::Duration,
}
impl ResourceResolver for RetryResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            let options = ExecutionOptions {
                model_retry_attempts: 3,
                model_retry_base_delay: self.delay,
                ..ExecutionOptions::default()
            };
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options,
            })
        })
    }
}
async fn retry_setup(
    delay: std::time::Duration,
) -> (
    Arc<Host>,
    ConnectionHandle,
    SessionRef,
    Arc<RetryModel>,
    mpsc::UnboundedReceiver<usize>,
) {
    let (starts, started) = mpsc::unbounded_channel();
    let model = Arc::new(RetryModel {
        descriptor: ModelDescriptor::new(
            "test/retry".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        calls: std::sync::atomic::AtomicUsize::new(0),
        starts,
        release: Arc::new(tokio::sync::Semaphore::new(0)),
    });
    let host = Host::memory(Arc::new(RetryResources {
        model: model.clone(),
        delay,
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let target = agent(&host, &connection.caller()).await;
    (host, connection, target, model, started)
}
async fn retry_state(
    sub: &mut EventSubscription,
    requesting: bool,
) -> (TurnIdentity, TurnStateData) {
    loop {
        if let ObservationEvent::State(StateEvent::Turn { source, data }) = next(sub).await
            && data.model_retry.as_ref().is_some_and(|retry| {
                matches!(retry.phase, ModelRetryPhase::Requesting) == requesting
            })
        {
            return (source, data);
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_wait_cancel_clears_authoritative_state_without_starting_attempt_two() {
    let (host, connection, target, model, mut starts) =
        retry_setup(std::time::Duration::from_secs(30)).await;
    let caller = connection.caller();
    let mut state = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::State],
    );
    send(&host, &caller, &target, MessageMode::FollowUp);
    assert_eq!(starts.recv().await, Some(1));
    let (source, waiting) = retry_state(&mut state, false).await;
    let retry = waiting.model_retry.unwrap();
    assert_eq!(
        (retry.response_index, retry.attempt, retry.max_attempts),
        (0, 2, 3)
    );
    assert_eq!(waiting.model_attempts, vec![1]);
    assert_eq!(waiting.phase, Some(TurnPhase::Model));
    let baseline = subscribe(
        &host,
        &caller,
        SubscriptionScope::Turn {
            session_id: target.session_id.clone(),
            turn_id: source.turn_id.clone(),
        },
        &[EventCategory::State],
    );
    assert!(
        baseline
            .result()
            .initial_states
            .iter()
            .any(|s| matches!(s,StateEvent::Turn {data,..} if data.model_retry.is_some()))
    );
    assert!(
        host.turn_info(&caller, &target, &source.turn_id)
            .unwrap()
            .model_retry
            .is_some()
    );
    host.cancel_turn(&caller, &target, &source.turn_id, false)
        .unwrap();
    loop {
        if let ObservationEvent::State(StateEvent::Turn { data, .. }) = next(&mut state).await
            && data.state == TurnState::Finished
        {
            assert!(data.model_retry.is_none());
            assert_eq!(data.model_attempts, vec![1]);
            break;
        }
    }
    assert_eq!(model.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_requesting_and_clear_use_original_response_and_actual_attempts() {
    let (host, connection, target, model, mut starts) =
        retry_setup(std::time::Duration::from_millis(1)).await;
    let caller = connection.caller();
    let mut state = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::State],
    );
    send(&host, &caller, &target, MessageMode::FollowUp);
    assert_eq!(starts.recv().await, Some(1));
    assert_eq!(starts.recv().await, Some(2));
    let (source, requesting) = retry_state(&mut state, true).await;
    assert_eq!(requesting.model_attempts, vec![2]);
    assert_eq!(requesting.model_retry.as_ref().unwrap().response_index, 0);
    model.release.add_permits(1);
    loop {
        if let ObservationEvent::State(StateEvent::Turn { data, .. }) = next(&mut state).await
            && data.state == TurnState::Finished
        {
            assert!(data.model_retry.is_none());
            assert_eq!(data.requests, 1);
            assert_eq!(data.model_attempts, vec![2]);
            break;
        }
    }
    assert_eq!(
        host.turn_output_read(&caller, &target, &source.turn_id, 0, &read(0, 9, 100))
            .unwrap()
            .data
            .text,
        "recovered"
    );
}

struct RequestInputTool {
    spec: minicore_host_runtime::tools::ToolSpec,
}
impl minicore_host_runtime::tools::Tool for RequestInputTool {
    fn spec(&self) -> &minicore_host_runtime::tools::ToolSpec {
        &self.spec
    }
    fn execute<'a>(
        &'a self,
        _: minicore_host_runtime::tools::ToolInvocation,
        _: minicore_host_runtime::tools::ToolContext,
    ) -> minicore_host_runtime::tools::ToolFuture<'a> {
        Box::pin(async {
            Ok(
                minicore_host_runtime::tools::ToolExecutionOutcome::RequestInput(
                    minicore_host_runtime::tools::ToolInputRequest::new(
                        "choose",
                        vec![minicore_host_runtime::Text::new("one")],
                        minicore_host_runtime::tools::ToolInputAnswerKind::SingleChoice,
                    )
                    .unwrap(),
                ),
            )
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interaction_observation_uses_actual_request_and_cancel_settlement() {
    let tool = Arc::new(RequestInputTool {
        spec: minicore_host_runtime::tools::ToolSpec::new(
            "read_fixture".parse().unwrap(),
            "ask fixture",
            serde_json::json!({"type":"object"}),
        )
        .unwrap(),
    });
    let (host, connection, events, mut starts) = setup(1, Some(tool));
    let caller = connection.caller();
    let target = agent(&host, &caller).await;
    let mut sub = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[
            EventCategory::Interaction,
            EventCategory::State,
            EventCategory::Completion,
        ],
    );
    send(&host, &caller, &target, MessageMode::FollowUp);
    starts.recv().await.unwrap();
    call_file(&events[0], std::path::Path::new("fixture"));
    let (source, id) = loop {
        if let ObservationEvent::Activity(ActivityEvent::InteractionRequested {
            source,
            interaction_id,
            data,
        }) = next(&mut sub).await
        {
            assert!(
                matches!(data,InteractionRequestData::ToolInput {request} if request.prompt.as_str()=="choose")
            );
            break (source, interaction_id);
        }
    };
    assert_eq!(id.as_str().len(), 20);
    assert!(!id.as_str().starts_with("int_"));
    let mut states = host.session_states(&caller, &target).unwrap();
    while states
        .snapshot()
        .unwrap()
        .data
        .pending_interactions
        .is_empty()
    {
        states.changed().await.unwrap();
    }
    let baseline = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::State],
    );
    assert!(baseline.result().initial_states.iter().any(|s|matches!(s,StateEvent::Session {data,..} if data.pending_interactions.iter().any(|p|p.interaction_id==id))));
    host.cancel_turn(&caller, &target, &source.turn.turn_id, false)
        .unwrap();
    loop {
        if let ObservationEvent::Activity(ActivityEvent::InteractionResolved {
            source: resolved,
            interaction_id,
            data,
        }) = next(&mut sub).await
        {
            assert_eq!(resolved, source);
            assert_eq!(interaction_id, id);
            assert_eq!(data.resolution, InteractionResolution::Cancelled);
            break;
        }
    }
    finish(&mut sub).await;
    assert!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .pending_interactions
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_pages_read_real_saved_content_without_recreating_old_execution() {
    let directory = tempfile::tempdir().unwrap();
    let (events, receiver) = mpsc::unbounded_channel();
    let (started, _starts) = mpsc::unbounded_channel();
    let model = Arc::new(ScriptedModel {
        descriptor: ModelDescriptor::new(
            "test/history".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(VecDeque::from([receiver])),
        started,
    });
    let resources: Arc<dyn ResourceResolver> = Arc::new(Resources { model, tool: None });
    let host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "history fixture".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let receipt = host
        .create_agent_async(
            &caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "saved".into(),
                mode: StorageMode::Persistent,
                config: AgentConfig::default(),
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    let mut completed = subscribe(
        &host,
        &caller,
        session_scope(&target),
        &[EventCategory::Completion],
    );
    send(&host, &caller, &target, MessageMode::FollowUp);
    complete(&events, "saved真实正文");
    let original = finish(&mut completed).await;
    let first = host
        .history_read(
            &caller,
            &target,
            &HistoryReadRequest {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert!(matches!(&first.data.items[0],HistoryItemData::User {text, ..} if text=="observe"));
    let old_cursor = first.data.next_cursor.clone().unwrap();
    let second = host
        .history_read(
            &caller,
            &target,
            &HistoryReadRequest {
                cursor: Some(old_cursor.clone()),
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(&second.data.items[0],HistoryItemData::Assistant {response} if matches!(&response.content[0],minicore_host_runtime::model::AssistantPart::Text(text) if text.as_ref()=="saved真实正文"))
    );
    assert!(second.data.next_cursor.is_none());
    let mut host_states = subscribe(
        &host,
        &caller,
        SubscriptionScope::Host {
            host_id: host.info(&caller).unwrap().host_id,
        },
        &[EventCategory::State],
    );
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    loop {
        if let ObservationEvent::State(StateEvent::Host { data, .. }) = next(&mut host_states).await
            && data.lifecycle == HostLifecycle::Stopped
        {
            break;
        }
    }
    drop(host_states);
    drop(completed);
    drop(connection);
    drop(host);
    let reopened = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "ignored".into(),
        },
        resources,
    )
    .await
    .unwrap();
    let operator = reopened.connect().unwrap();
    let reader = reopened.connect().unwrap();
    let caller = reader.caller();
    assert_eq!(
        reopened
            .session_state(&caller, &target)
            .unwrap()
            .data
            .residency,
        Residency::Unloaded
    );
    assert_eq!(
        reopened
            .history_read(
                &caller,
                &target,
                &HistoryReadRequest {
                    cursor: Some(old_cursor),
                    limit: 1
                }
            )
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionHistoryCursorInvalid)
    );
    let page = reopened
        .history_read(
            &caller,
            &target,
            &HistoryReadRequest {
                cursor: None,
                limit: 20,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.data.items.len(), 2);
    assert!(page.data.next_cursor.is_none());
    assert_eq!(
        reopened
            .session_state(&caller, &target)
            .unwrap()
            .data
            .residency,
        Residency::Unloaded
    );
    assert!(reopened.turns(&caller, &target).unwrap().is_empty());
    assert_eq!(
        reopened
            .turn_output_read(&caller, &target, &original.turn_id, 0, &read(0, 4, 4))
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionTurnNotFound)
    );
    let wire = serde_json::to_value(&page).unwrap();
    assert!(!wire.to_string().contains("loop_id"));
    assert!(wire.get("load_id").is_none());
    drop(operator);
}

#[tokio::test]
async fn session_preparation_is_not_published_as_a_registered_child() {
    let (host, connection, _events, _starts) = setup(0, None);
    let caller = connection.caller();
    let mut sub = subscribe(
        &host,
        &caller,
        SubscriptionScope::Host {
            host_id: host.info(&caller).unwrap().host_id,
        },
        &[EventCategory::State],
    );
    let target = agent(&host, &caller).await;
    host.events_unsubscribe(
        &caller,
        &UnsubscribeRequest {
            subscription_id: sub.result().subscription_id.clone(),
        },
    )
    .unwrap();
    let mut states = 0;
    while let Some(ObservationNotification::Event(push)) = sub.recv().await.unwrap() {
        if let ObservationEvent::State(StateEvent::Session { source, .. }) = push.event {
            assert_eq!(source.session_id, target.session_id);
            states += 1;
        }
    }
    assert_eq!(
        states, 1,
        "only the registered Session publishes its initial state"
    );
}

#[tokio::test]
async fn agent_scope_covers_later_direct_sessions_without_rebuilding() {
    let (host, connection, _events, _starts) = setup(0, None);
    let caller = connection.caller();
    let first = agent(&host, &caller).await;
    let host_id = host.info(&caller).unwrap().host_id;
    let mut sub = subscribe(
        &host,
        &caller,
        SubscriptionScope::Agent {
            host_id: host_id.clone(),
            agent_id: first.agent_id.clone(),
        },
        &[EventCategory::State],
    );
    assert!(sub.result().initial_states.iter().any(|state|matches!(state,StateEvent::Session {source,..} if source.session_id==first.session_id)));
    let receipt = host
        .create_session_async(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: AgentRef {
                    host_id,
                    agent_id: first.agent_id.clone(),
                },
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    match next(&mut sub).await {
        ObservationEvent::State(StateEvent::Session { source, .. }) => {
            assert_eq!(source.session_id, receipt.session_id.unwrap());
            assert_eq!(source.agent_id, first.agent_id);
        }
        other => panic!("unexpected later child event: {other:?}"),
    }
}
