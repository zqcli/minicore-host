//! Real Host/Runtime, tools and journal with explicitly scripted model streams.
mod common;
mod observation_common;
mod os_pipe;

use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary},
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelError, ModelErrorKind, ModelEvent,
        ModelFinishReason, ModelRequest, ModelStartFuture,
    },
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSpec,
    },
};
use minicore_host_transport::{HostClient, SessionClient};
use observation_common::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

const RAW_ID: &str = "provider-reused-id";

struct EchoFixture {
    spec: ToolSpec,
    calls: AtomicUsize,
}
impl Tool for EchoFixture {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let text = invocation.arguments["text"].as_str().unwrap();
            assert!(ToolOutputSink::current().unwrap().emit("stdout", text));
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new(text).unwrap(),
            ))
        })
    }
}

struct RetryFixture {
    inner: Arc<dyn Model>,
    reject_next_start: AtomicBool,
}
impl Model for RetryFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        self.inner.descriptor()
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        if self.reject_next_start.swap(false, Ordering::SeqCst) {
            return Box::pin(async {
                Err(ModelError::not_started(
                    ModelErrorKind::ProviderUnavailable,
                    None,
                    DiagnosticSummary::new(
                        DiagnosticCode::ModelUnavailable,
                        DiagnosticCategory::Model,
                        minicore_host_runtime::Text::new("scripted pre-delivery retry"),
                        true,
                    )
                    .unwrap(),
                ))
            });
        }
        self.inner.start(request, context)
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    resources: Arc<dyn ResourceResolver>,
    client: HostClient,
    session: SessionClient,
    events: Vec<Events>,
    starts: mpsc::UnboundedReceiver<()>,
    tool: Arc<EchoFixture>,
    model: Arc<RetryFixture>,
}
impl Fixture {
    async fn new(mode: StorageMode, rounds: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let tool = Arc::new(EchoFixture {
            spec: ToolSpec::new(
                "echo_fixture".parse().unwrap(),
                "Explicit scripted identity regression fixture",
                serde_json::json!({"type":"object"}),
            )
            .unwrap(),
            calls: AtomicUsize::new(0),
        });
        let (source, events, starts) = resources(rounds, None);
        let model = Arc::new(RetryFixture {
            inner: source
                .resolve(&SessionConfig::default())
                .await
                .unwrap()
                .execution
                .model()
                .clone(),
            reject_next_start: AtomicBool::new(false),
        });
        let resources: Arc<dyn ResourceResolver> = Arc::new(Resources {
            model: model.clone(),
            tool: Some(tool.clone()),
        });
        let host = if mode == StorageMode::Memory {
            Host::memory(resources.clone()).unwrap()
        } else {
            Host::open_persistent(
                directory.path(),
                HostSettings {
                    name: "identity fixture".into(),
                },
                resources.clone(),
            )
            .await
            .unwrap()
        };
        let client = HostClient::in_process(host).unwrap();
        let mut request = create_request();
        request.mode = mode;
        let made = client.create_agent(request, options(3000)).await.unwrap();
        let session = client.session(SessionRef {
            agent_id: made.agent_id.unwrap(),
            session_id: made.session_id.unwrap(),
        });
        Self {
            directory,
            resources,
            client,
            session,
            events,
            starts,
            tool,
            model,
        }
    }
    async fn started(&mut self) {
        tokio::time::timeout(Duration::from_secs(3), self.starts.recv())
            .await
            .unwrap()
            .unwrap();
    }
}

fn invocation(events: &Events, id: &str, text: &str) {
    let id: minicore_host_runtime::ToolCallId = id.parse().unwrap();
    events
        .send(ModelEvent::ToolCallStart {
            tool_call_id: id.clone(),
            tool_name: "echo_fixture".parse().unwrap(),
        })
        .unwrap();
    events
        .send(ModelEvent::tool_call_arguments_delta(
            id.clone(),
            serde_json::json!({"text":text}).to_string(),
        ))
        .unwrap();
    events
        .send(ModelEvent::ToolCallEnd { tool_call_id: id })
        .unwrap();
}
fn call(events: &Events, text: &str) {
    invocation(events, RAW_ID, text);
    events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        })
        .unwrap();
}
fn target(turn_id: TurnId) -> ToolCallRef {
    ToolCallRef {
        turn_id,
        tool_call_id: ToolCallId(RAW_ID.into()),
    }
}
async fn assert_original(session: &SessionClient, target: ToolCallRef, text: &str) {
    let record = session
        .toolcall(target.clone(), options(3000))
        .await
        .unwrap();
    assert_eq!(record.source.tool_call_id.as_str(), RAW_ID);
    assert_eq!(record.data.state.response_index, 0);
    assert_eq!(record.data.arguments["text"], text);
    assert_eq!(record.data.result.unwrap().output.content().as_str(), text);
    let output = session
        .tool_output(
            target,
            TextReadRequest {
                channel: "stdout".into(),
                start: 0,
                end: text.len() as u64,
                max_bytes: 100,
            },
            options(3000),
        )
        .await
        .unwrap();
    assert_eq!(output.data.text, text);
}
async fn stop(client: &HostClient) {
    client
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Force,
            },
            options(3000),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while client.info(options(3000)).await.unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("force shutdown settles the failed Turn and its save");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_loop_repeated_id_preserves_original_memory_tool() {
    reject_duplicate(StorageMode::Memory, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn internal_continuation_repeated_id_preserves_original_memory_tool() {
    reject_duplicate(StorageMode::Memory, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_loop_repeated_id_settles_persistent_turn_and_shutdown() {
    reject_duplicate(StorageMode::Persistent, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn internal_continuation_repeated_id_settles_persistent_turn_and_shutdown() {
    reject_duplicate(StorageMode::Persistent, true).await;
}

async fn reject_duplicate(mode: StorageMode, continuation: bool) {
    let mut fixture = Fixture::new(mode, if continuation { 4 } else { 3 }).await;
    fixture
        .session
        .send(input("identity check"), options(3000))
        .await
        .unwrap();
    fixture.started().await;
    let turn_id = running(&fixture.session).await;
    call(&fixture.events[0], "first");
    fixture.started().await;
    assert_original(&fixture.session, target(turn_id.clone()), "first").await;
    let second = if continuation {
        let mut steer = input("same public Turn, next internal Loop");
        steer.mode = MessageMode::Steer;
        fixture.session.send(steer, options(3000)).await.unwrap();
        fixture.events[1]
            .send(ModelEvent::text_delta("first Loop finished"))
            .unwrap();
        finish(&fixture.events[1]);
        fixture.started().await;
        2
    } else {
        1
    };
    // Real text and a distinct completed call preceding the invalid start must
    // survive as partial content, without executing or publishing either call.
    fixture.events[second]
        .send(ModelEvent::text_delta("accepted partial prefix"))
        .unwrap();
    invocation(
        &fixture.events[second],
        "unexecuted-partial-call",
        "partial only",
    );
    // A further response is scripted only to make the unfixed behavior finish
    // deterministically. Rejection must prevent that request entirely.
    fixture.events[second + 1]
        .send(ModelEvent::text_delta("must not request this response"))
        .unwrap();
    finish(&fixture.events[second + 1]);
    fixture.events[second]
        .send(ModelEvent::ToolCallStart {
            tool_call_id: RAW_ID.parse().unwrap(),
            tool_name: "echo_fixture".parse().unwrap(),
        })
        .unwrap();
    // The stream may already have been dropped at the rejected start.
    for event in [
        ModelEvent::tool_call_arguments_delta(
            RAW_ID.parse().unwrap(),
            serde_json::json!({"text":"second"}).to_string(),
        ),
        ModelEvent::ToolCallEnd {
            tool_call_id: RAW_ID.parse().unwrap(),
        },
        ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        },
    ] {
        let _ = fixture.events[second].send(event);
    }
    let result = match fixture
        .session
        .wait_turn(turn_id.clone(), options(3000))
        .await
    {
        Ok(result) => result,
        Err(error) => panic!(
            "duplicate response must settle: {error:?}; original state: {:?}",
            fixture.session.turn(turn_id.clone(), options(3000)).await
        ),
    };
    assert_original(&fixture.session, target(turn_id.clone()), "first").await;
    let info = result.data.info.as_ref().unwrap();
    assert_eq!(info.state, TurnState::Finished);
    assert_eq!(info.outcome, Some(TurnOutcome::Failed));
    assert_eq!(
        info.persistence,
        if mode == StorageMode::Memory {
            PersistenceState::NotApplicable
        } else {
            PersistenceState::Persisted
        }
    );
    assert!(info.persistence_fault.is_none());
    let fault = info.fault.as_ref().unwrap();
    assert_eq!(fault.kind(), Some(FaultKind::TurnModelUpstreamUnclassified));
    assert!(
        fault
            .message()
            .contains("ToolCallId already accepted by this Turn")
    );
    assert_eq!(
        info.model_attempts,
        vec![1; second + 1],
        "protocol failure is never retried"
    );
    assert_eq!(fixture.tool.calls.load(Ordering::SeqCst), 1);
    assert!(
        fixture.starts.try_recv().is_err(),
        "invalid response cannot issue another request"
    );
    let calls: Vec<_> = result
        .data
        .history
        .iter()
        .flat_map(|entry| match &entry.item {
            HistoryItemData::Assistant { response } => response
                .content
                .iter()
                .filter_map(|part| part.as_tool_call())
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "reconciliation must not accept an invalid report suffix"
    );
    assert_eq!(calls[0].tool_call_id().as_str(), RAW_ID);
    assert_eq!(result.data.partials.len(), 1);
    let partial = &result.data.partials[0];
    assert_eq!(partial.response_index, second as u64);
    assert!(
        partial.partial.incomplete_tool_calls.is_empty(),
        "rejected ID never reaches Runtime partial assembly"
    );
    let partial_calls: Vec<_> = partial
        .partial
        .content
        .iter()
        .filter_map(|part| part.as_tool_call())
        .collect();
    assert_eq!(partial_calls.len(), 1);
    assert_eq!(
        partial_calls[0].tool_call_id().as_str(),
        "unexecuted-partial-call"
    );
    assert!(
        serde_json::to_string(&partial.partial)
            .unwrap()
            .contains("accepted partial prefix")
    );
    assert!(
        fixture
            .session
            .toolcall(
                ToolCallRef {
                    turn_id: turn_id.clone(),
                    tool_call_id: ToolCallId("unexecuted-partial-call".into()),
                },
                options(3000)
            )
            .await
            .is_err()
    );
    let history = fixture
        .session
        .history(
            HistoryReadRequest {
                direction: minicore_host_protocol::HistoryDirection::Forward,
                cursor: None,
                limit: 100,
            },
            options(3000),
        )
        .await
        .unwrap();
    stop(&fixture.client).await;
    if mode == StorageMode::Persistent {
        let target = fixture.session.target().clone();
        let reopened = Host::open_persistent(
            fixture.directory.path(),
            HostSettings {
                name: "reopened identity fixture".into(),
            },
            fixture.resources.clone(),
        )
        .await
        .unwrap();
        let client = HostClient::in_process(reopened).unwrap();
        let session = client.session(target);
        let cold = session
            .history(
                HistoryReadRequest {
                    direction: minicore_host_protocol::HistoryDirection::Forward,
                    cursor: None,
                    limit: 100,
                },
                options(3000),
            )
            .await
            .unwrap();
        assert_eq!(value(history.data), value(cold.data));
        let saved = session.turn_result(turn_id, options(3000)).await.unwrap();
        assert_eq!(
            saved.data.saved_terminal.unwrap().outcome,
            TurnOutcome::Failed
        );
        assert_eq!(value(saved.data.partials), value(result.data.partials));
        stop(&client).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn different_public_turns_allow_same_raw_tool_id() {
    let mut fixture = Fixture::new(StorageMode::Persistent, 4).await;
    let mut turns = Vec::new();
    for (index, text) in ["first", "second"].into_iter().enumerate() {
        fixture
            .session
            .send(input(text), options(3000))
            .await
            .unwrap();
        fixture.started().await;
        let turn = running(&fixture.session).await;
        call(&fixture.events[index * 2], text);
        fixture.started().await;
        fixture.events[index * 2 + 1]
            .send(ModelEvent::text_delta("done"))
            .unwrap();
        finish(&fixture.events[index * 2 + 1]);
        let result = fixture
            .session
            .wait_turn(turn.clone(), options(3000))
            .await
            .unwrap();
        assert_eq!(
            result.data.info.unwrap().outcome,
            Some(TurnOutcome::Completed)
        );
        turns.push(turn);
    }
    assert_ne!(turns[0], turns[1]);
    assert_eq!(fixture.tool.calls.load(Ordering::SeqCst), 2);
    assert_original(&fixture.session, target(turns[0].clone()), "first").await;
    assert_original(&fixture.session, target(turns[1].clone()), "second").await;
    stop(&fixture.client).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_id_and_pre_delivery_retry_do_not_reserve_a_tool_identity() {
    let mut fixture = Fixture::new(StorageMode::Persistent, 3).await;
    fixture
        .session
        .send(input("incomplete response"), options(3000))
        .await
        .unwrap();
    fixture.started().await;
    let partial_turn = running(&fixture.session).await;
    invocation(&fixture.events[0], RAW_ID, "never executed");
    // Closing the actual stream before Finish yields a failed partial; Runtime
    // correctly does not retry a request that has already delivered events.
    fixture.events.remove(0);
    let partial = fixture
        .session
        .wait_turn(partial_turn.clone(), options(3000))
        .await
        .unwrap();
    assert_eq!(
        partial.data.info.unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    assert_eq!(partial.data.partials.len(), 1);
    assert_eq!(fixture.tool.calls.load(Ordering::SeqCst), 0);
    assert!(
        fixture
            .session
            .toolcall(target(partial_turn), options(3000))
            .await
            .is_err()
    );
    fixture
        .model
        .reject_next_start
        .store(true, Ordering::SeqCst);
    fixture
        .session
        .send(input("explicit next Turn"), options(3000))
        .await
        .unwrap();
    fixture
        .session
        .resume(control(), options(3000))
        .await
        .unwrap();
    fixture.started().await;
    let next_turn = running(&fixture.session).await;
    call(&fixture.events[0], "first");
    fixture.started().await;
    fixture.events[1]
        .send(ModelEvent::text_delta("done"))
        .unwrap();
    finish(&fixture.events[1]);
    let result = fixture
        .session
        .wait_turn(next_turn.clone(), options(3000))
        .await
        .unwrap();
    let info = result.data.info.unwrap();
    assert_eq!(info.outcome, Some(TurnOutcome::Completed));
    assert_eq!(info.model_attempts, vec![2, 1]);
    assert_eq!(info.persistence, PersistenceState::Persisted);
    assert_eq!(fixture.tool.calls.load(Ordering::SeqCst), 1);
    assert_original(&fixture.session, target(next_turn), "first").await;
    stop(&fixture.client).await;
}
