//! Answers reach actual Runtime ToolInput waiting points, not projections.
mod common;
mod observation_common;
mod os_pipe;
mod wss_common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    model::{ModelEvent, ModelFinishReason},
    tools::*,
};
use minicore_host_transport::*;
use observation_common::*;
use std::{sync::Arc, time::Duration};

struct InputTool {
    spec: ToolSpec,
}
impl Tool for InputTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        Box::pin(async {
            Ok(ToolExecutionOutcome::RequestInput(
                ToolInputRequest::new(
                    "choose actual input",
                    vec![
                        minicore_host_runtime::Text::new("one"),
                        minicore_host_runtime::Text::new("two"),
                    ],
                    ToolInputAnswerKind::SingleChoice,
                )
                .unwrap(),
            ))
        })
    }
}
fn tool() -> Arc<dyn Tool> {
    Arc::new(InputTool {
        spec: ToolSpec::new(
            "ask_fixture".parse().unwrap(),
            "request actual input",
            serde_json::json!({"type":"object"}),
        )
        .unwrap(),
    })
}
fn call(events: &Events) {
    let id: minicore_host_runtime::ToolCallId = "same-call-id".parse().unwrap();
    events
        .send(ModelEvent::ToolCallStart {
            tool_call_id: id.clone(),
            tool_name: "ask_fixture".parse().unwrap(),
        })
        .unwrap();
    events
        .send(ModelEvent::tool_call_arguments_delta(id.clone(), "{}"))
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
async fn pending(sub: &mut ClientSubscription) -> (ToolCallIdentity, InteractionId) {
    loop {
        if let ObservationEvent::Activity(ActivityEvent::InteractionRequested {
            source,
            interaction_id,
            ..
        }) = next(sub).await
        {
            return (source, interaction_id);
        }
    }
}
fn answer(
    source: &ToolCallIdentity,
    id: InteractionId,
) -> InteractionAnswerRequest<InteractionAnswerData> {
    InteractionAnswerRequest {
        operation_id: OperationId::new().unwrap(),
        turn_id: source.turn.turn_id.clone(),
        load_id: source.turn.load_id.clone().unwrap(),
        interaction_id: id,
        answer: InteractionAnswerData::ToolInput {
            answer: ToolInputAnswer::Choice { index: 1 },
        },
    }
}
async fn observed(client: &HostClient, session: &SessionClient) -> ClientSubscription {
    client
        .subscribe(
            request(
                scope(session.target()),
                &[EventCategory::Interaction, EventCategory::Completion],
            ),
            options(2000),
        )
        .await
        .unwrap()
}
fn kind(error: CallError) -> Option<FaultKind> {
    match error {
        CallError::Business(fault) => fault.kind(),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_answer_survives_operator_replacement_preserves_pause_and_retains_runtime_result() {
    for transport in 0..3 {
        let (host, events, mut started) = setup(2, Some(tool()));
        let server = if transport == 2 {
            Some(wss_common::Server::start(host.clone()).await)
        } else {
            None
        };
        let (operator, pipe) = match &server {
            Some(server) => (server.client(wss_common::FIRST).await.unwrap(), None),
            None if transport == 1 => {
                let (client, task) = stdio_pipes(host.clone()).await;
                (client, Some(task))
            }
            _ => (HostClient::in_process(host.clone()).unwrap(), None),
        };
        let session = create(&operator).await;
        let mut sub = observed(&operator, &session).await;
        session.send(input("ask"), options(2000)).await.unwrap();
        started.recv().await.unwrap();
        call(&events[0]);
        let (source, id) = pending(&mut sub).await;
        assert_eq!(id.as_str().len(), 20);
        let request = answer(&source, id.clone());
        let mut wrong = request.clone();
        wrong.load_id = LoadId::new().unwrap();
        assert_eq!(
            kind(
                session
                    .answer_interaction(wrong, options(2000))
                    .await
                    .unwrap_err()
            ),
            Some(FaultKind::SessionOwnershipMismatch)
        );
        let mut wrong = request.clone();
        wrong.turn_id = TurnId::new().unwrap();
        assert_eq!(
            kind(
                session
                    .answer_interaction(wrong, options(2000))
                    .await
                    .unwrap_err()
            ),
            Some(FaultKind::SessionTurnNotFound)
        );
        let mut wrong = request.clone();
        wrong.interaction_id = InteractionId::new().unwrap();
        assert_eq!(
            kind(
                session
                    .answer_interaction(wrong, options(2000))
                    .await
                    .unwrap_err()
            ),
            Some(FaultKind::SessionInteractionNotPending)
        );
        let mut wrong = request.clone();
        wrong.answer = InteractionAnswerData::ToolInput {
            answer: ToolInputAnswer::Choice { index: 99 },
        };
        assert_eq!(
            kind(
                session
                    .answer_interaction(wrong, options(2000))
                    .await
                    .unwrap_err()
            ),
            Some(FaultKind::SessionInteractionAnswerInvalid)
        );
        let observer = HostClient::in_process(host.clone()).unwrap();
        assert_eq!(
            kind(
                observer
                    .session(session.target().clone())
                    .answer_interaction(request.clone(), options(2000))
                    .await
                    .unwrap_err()
            ),
            Some(FaultKind::HostAccessReadOnly)
        );
        session
            .pause(common::control(), options(2000))
            .await
            .unwrap();
        operator.close();
        tokio::time::timeout(Duration::from_secs(3), async {
            while observer.info(options(2000)).await.unwrap().control.occupied {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (replacement, replacement_pipe) = match &server {
            Some(server) => (server.client(wss_common::FIRST).await.unwrap(), None),
            None if transport == 1 => {
                let (client, task) = stdio_pipes(host.clone()).await;
                (client, Some(task))
            }
            _ => (HostClient::in_process(host.clone()).unwrap(), None),
        };
        assert!(replacement.initial_info().can_operate);
        let target = replacement.session(session.target().clone());
        let mut sub = observed(&replacement, &target).await;
        let receipt = target
            .answer_interaction(request.clone(), options(2000))
            .await
            .unwrap();
        assert_eq!(receipt.result.status, InteractionAnswerStatus::Answered);
        assert_eq!(receipt.interaction_id, id);
        assert!(
            matches!(replacement.receipt(request.operation_id.clone(), options(2000)).await.unwrap(), Receipt::InteractionAnswer(actual) if actual==receipt)
        );
        assert_eq!(
            target.state(options(2000)).await.unwrap().data.scheduling,
            Scheduling::Paused
        );
        assert!(
            target
                .answer_interaction(request, options(2000))
                .await
                .is_err()
        );
        started.recv().await.unwrap();
        events[1]
            .send(ModelEvent::text_delta("after actual input"))
            .unwrap();
        finish(&events[1]);
        let mut resolutions = 0;
        let mut tool_result = None;
        loop {
            match next(&mut sub).await {
                ObservationEvent::Activity(ActivityEvent::InteractionResolved {
                    interaction_id,
                    data,
                    ..
                }) => {
                    assert_eq!(interaction_id, id);
                    assert_eq!(data.resolution, InteractionResolution::Answered);
                    resolutions += 1;
                }
                ObservationEvent::Activity(ActivityEvent::ToolFinished { data, .. }) => {
                    tool_result = Some(data.result);
                }
                ObservationEvent::Activity(ActivityEvent::TurnFinished {
                    source: ended, ..
                }) => {
                    assert_eq!(ended, source.turn);
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(resolutions, 1);
        let tool_result = tool_result.unwrap();
        assert_eq!(tool_result.outcome, ToolResultOutcome::InputProvided);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(tool_result.output.content().as_str())
                .unwrap()["choice"],
            "two"
        );
        assert_eq!(
            target.state(options(2000)).await.unwrap().data.scheduling,
            Scheduling::Paused
        );
        replacement.close();
        observer.close();
        if let Some(task) = replacement_pipe {
            task.await.unwrap().unwrap();
        }
        if let Some(task) = pipe {
            task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_before_answer_rejects_it_and_answer_before_cancel_keeps_the_answered_fact() {
    for answer_first in [false, true] {
        let (host, events, mut started) = setup(2, Some(tool()));
        let (client, server) = stdio_pipes(host).await;
        let session = create(&client).await;
        let mut sub = observed(&client, &session).await;
        session.send(input("order"), options(2000)).await.unwrap();
        started.recv().await.unwrap();
        call(&events[0]);
        let (source, id) = pending(&mut sub).await;
        let request = answer(&source, id.clone());
        if answer_first {
            session
                .answer_interaction(request.clone(), options(2000))
                .await
                .unwrap();
        }
        session
            .cancel_turn(
                TurnCancelRequest {
                    operation_id: OperationId::new().unwrap(),
                    turn_id: source.turn.turn_id.clone(),
                    keep_inbox: false,
                },
                options(2000),
            )
            .await
            .unwrap();
        if !answer_first {
            assert!(
                session
                    .answer_interaction(request, options(2000))
                    .await
                    .is_err()
            );
        }
        let mut resolutions = Vec::new();
        loop {
            match next(&mut sub).await {
                ObservationEvent::Activity(ActivityEvent::InteractionResolved {
                    interaction_id,
                    data,
                    ..
                }) => {
                    assert_eq!(interaction_id, id);
                    resolutions.push(data.resolution);
                }
                ObservationEvent::Activity(ActivityEvent::TurnFinished {
                    source: actual,
                    data,
                }) => {
                    assert_eq!(actual, source.turn);
                    assert_eq!(data.outcome, Some(TurnOutcome::Cancelled));
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(
            resolutions,
            vec![if answer_first {
                InteractionResolution::Answered
            } else {
                InteractionResolution::Cancelled
            }]
        );
        client.close();
        server.await.unwrap().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_session_interactions_with_reused_tool_ids_answer_only_the_named_wait() {
    let (host, events, mut started) = setup(4, Some(tool()));
    let (client, server) = stdio_pipes(host).await;
    let first = create(&client).await;
    let second = create(&client).await;
    let mut a = observed(&client, &first).await;
    let mut b = observed(&client, &second).await;
    first.send(input("first"), options(2000)).await.unwrap();
    started.recv().await.unwrap();
    call(&events[0]);
    let (source_a, id_a) = pending(&mut a).await;
    second.send(input("second"), options(2000)).await.unwrap();
    started.recv().await.unwrap();
    call(&events[1]);
    let (source_b, id_b) = pending(&mut b).await;
    assert_eq!(source_a.tool_call_id, source_b.tool_call_id);
    assert_ne!(id_a, id_b);
    let mut crossed = answer(&source_b, id_b.clone());
    crossed.interaction_id = id_a.clone();
    assert_eq!(
        kind(
            second
                .answer_interaction(crossed, options(2000))
                .await
                .unwrap_err()
        ),
        Some(FaultKind::SessionInteractionNotPending)
    );
    first
        .answer_interaction(answer(&source_a, id_a), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    assert!(
        second
            .state(options(2000))
            .await
            .unwrap()
            .data
            .pending_interactions
            .iter()
            .any(|pending| pending.interaction_id == id_b)
    );
    second
        .answer_interaction(answer(&source_b, id_b), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    finish(&events[2]);
    finish(&events[3]);
    turn_finished(&mut a).await;
    turn_finished(&mut b).await;
    client.close();
    server.await.unwrap().unwrap();
}

#[test]
fn answer_payload_reuses_strict_runtime_types_and_rejects_unregistered_shapes() {
    for value in [
        serde_json::json!({"kind":"approval","decision":"always"}),
        serde_json::json!({"kind":"approval","decision":"allow_once","permission":"admin"}),
        serde_json::json!({"kind":"tool_input","answer":{"kind":"choice","data":{"index":-1}}}),
        serde_json::json!({"kind":"tool_input","answer":{"kind":"choice","data":{"index":0,"other":true}}}),
        serde_json::json!({"kind":"unknown","answer":true}),
    ] {
        assert!(serde_json::from_value::<InteractionAnswerData>(value).is_err());
    }
}

struct ApprovalPolicy;
impl ToolPolicy for ApprovalPolicy {
    fn decide<'a>(&'a self, _: ToolPolicyRequest) -> ToolPolicyFuture<'a> {
        Box::pin(async {
            ToolDecision::require_approval(
                ApprovalRequest::new("permit actual fixture tool", ApprovalRisk::Low).unwrap(),
            )
        })
    }
}
struct CountingTool {
    spec: ToolSpec,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl Tool for CountingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("actually executed").unwrap(),
            ))
        })
    }
}
struct ApprovalResources {
    base: Arc<dyn ResourceResolver>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl ResourceResolver for ApprovalResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.base.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            let mut bound = self.base.resolve(config).await?;
            let mut tools = ToolSet::builder();
            tools.register(CountingTool {
                spec: ToolSpec::new(
                    "ask_fixture".parse().unwrap(),
                    "count actual execution",
                    serde_json::json!({"type":"object"}),
                )
                .unwrap(),
                calls: self.calls.clone(),
            });
            bound.execution = minicore_host_runtime::ExecutionConfig::new(
                bound.execution.model().clone(),
                bound.execution.reasoning(),
                tools.build().unwrap(),
                Some(Arc::new(ApprovalPolicy)),
                bound.execution.prompt().clone(),
            )
            .unwrap();
            Ok(bound)
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_answers_allow_or_deny_the_original_real_tool_without_new_permissions() {
    for decision in [ApprovalAnswer::AllowOnce, ApprovalAnswer::Deny] {
        let (base, events, mut started) = resources(2, None);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let host = Host::memory(Arc::new(ApprovalResources {
            base,
            calls: calls.clone(),
        }))
        .unwrap();
        let (client, server) = stdio_pipes(host).await;
        let session = create(&client).await;
        let mut sub = observed(&client, &session).await;
        session
            .send(input("approval"), options(2000))
            .await
            .unwrap();
        started.recv().await.unwrap();
        call(&events[0]);
        let (source, id) = loop {
            if let ObservationEvent::Activity(ActivityEvent::InteractionRequested {
                source,
                interaction_id,
                data,
            }) = next(&mut sub).await
            {
                assert!(matches!(data, InteractionRequestData::Approval { .. }));
                break (source, interaction_id);
            }
        };
        let mut request = answer(&source, id);
        assert_eq!(
            kind(
                session
                    .answer_interaction(request.clone(), options(2000))
                    .await
                    .unwrap_err()
            ),
            Some(FaultKind::SessionInteractionAnswerInvalid)
        );
        request.answer = InteractionAnswerData::Approval { decision };
        session
            .answer_interaction(request, options(2000))
            .await
            .unwrap();
        started.recv().await.unwrap();
        finish(&events[1]);
        let outcome = loop {
            if let ObservationEvent::Activity(ActivityEvent::ToolFinished { data, .. }) =
                next(&mut sub).await
            {
                break data.result.outcome;
            }
        };
        assert_eq!(
            outcome,
            if decision == ApprovalAnswer::AllowOnce {
                ToolResultOutcome::Success
            } else {
                ToolResultOutcome::Denied
            }
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(decision == ApprovalAnswer::AllowOnce)
        );
        turn_finished(&mut sub).await;
        client.close();
        server.await.unwrap().unwrap();
    }
}

struct DeadlineResources {
    base: Arc<dyn ResourceResolver>,
}
impl ResourceResolver for DeadlineResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.base.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            let mut bound = self.base.resolve(config).await?;
            bound.options.deadline = Some(tokio::time::Instant::now() + Duration::from_secs(2));
            Ok(bound)
        })
    }
}
#[derive(Clone, Copy)]
enum BeforeDeadline {
    Pending,
    Answer,
    Cancel,
}
async fn settle_at_original_deadline(approval: bool, before: BeforeDeadline) {
    let (base, events, mut started) = resources(2, (!approval).then(tool));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let base: Arc<dyn ResourceResolver> = if approval {
        Arc::new(ApprovalResources {
            base,
            calls: calls.clone(),
        })
    } else {
        base
    };
    let host = Host::memory(Arc::new(DeadlineResources { base })).unwrap();
    let (client, server) = stdio_pipes(host).await;
    let session = create(&client).await;
    let mut sub = observed(&client, &session).await;
    session
        .send(input("original deadline"), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    call(&events[0]);
    let (source, id) = pending(&mut sub).await;
    let mut request = answer(&source, id.clone());
    if approval {
        request.answer = InteractionAnswerData::Approval {
            decision: ApprovalAnswer::AllowOnce,
        };
    }
    match before {
        BeforeDeadline::Pending => {}
        BeforeDeadline::Answer => {
            session
                .answer_interaction(request.clone(), options(2000))
                .await
                .unwrap();
            // The answer is accepted at its actual waiting point. Leave the
            // next model request open so the same original deadline ends it.
            started.recv().await.unwrap();
        }
        BeforeDeadline::Cancel => {
            session
                .cancel_turn(
                    TurnCancelRequest {
                        operation_id: OperationId::new().unwrap(),
                        turn_id: source.turn.turn_id.clone(),
                        keep_inbox: false,
                    },
                    options(2000),
                )
                .await
                .unwrap();
        }
    }
    let mut resolutions = Vec::new();
    let mut tool_results = Vec::new();
    loop {
        match next(&mut sub).await {
            ObservationEvent::Activity(ActivityEvent::InteractionResolved {
                source: actual,
                interaction_id,
                data,
            }) => {
                assert_eq!(actual, source);
                assert_eq!(interaction_id, id);
                resolutions.push(serde_json::to_value(data.resolution).unwrap());
            }
            ObservationEvent::Activity(ActivityEvent::ToolFinished { data, .. }) => {
                tool_results.push(data.result.outcome);
            }
            ObservationEvent::Activity(ActivityEvent::TurnFinished {
                source: actual,
                data,
            }) => {
                assert_eq!(actual, source.turn);
                assert_eq!(data.outcome, Some(TurnOutcome::Cancelled));
                break;
            }
            _ => {}
        }
    }
    assert!(
        session
            .answer_interaction(request, options(2000))
            .await
            .is_err()
    );
    assert!(
        session
            .state(options(2000))
            .await
            .unwrap()
            .data
            .pending_interactions
            .is_empty()
    );
    let (resolution, tool_outcome) = match before {
        BeforeDeadline::Pending => ("expired", ToolResultOutcome::Cancelled),
        BeforeDeadline::Cancel => ("cancelled", ToolResultOutcome::Cancelled),
        BeforeDeadline::Answer => (
            "answered",
            if approval {
                ToolResultOutcome::Success
            } else {
                ToolResultOutcome::InputProvided
            },
        ),
    };
    assert_eq!(resolutions, [serde_json::json!(resolution)]);
    assert_eq!(tool_results, [tool_outcome]);
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        usize::from(approval && matches!(before, BeforeDeadline::Answer))
    );
    client.close();
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_deadline_expires_pending_approval_and_tool_input_once() {
    for approval in [false, true] {
        settle_at_original_deadline(approval, BeforeDeadline::Pending).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_deadline_does_not_replace_an_already_answered_interaction() {
    for approval in [false, true] {
        settle_at_original_deadline(approval, BeforeDeadline::Answer).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_cancel_before_deadline_remains_cancelled_for_both_interaction_kinds() {
    for approval in [false, true] {
        settle_at_original_deadline(approval, BeforeDeadline::Cancel).await;
    }
}
