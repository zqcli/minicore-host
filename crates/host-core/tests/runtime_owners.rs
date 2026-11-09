//! These tests use the real Host owners and AgentLoop. Only model responses are
//! scripted fixtures; no external provider or TUI validation is implied.
#[path = "support/input_context.rs"]
mod input_context;
use futures_util::{StreamExt, stream};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_runtime::{
    ExecutionConfig, HistoryItem, LoopOptions, Text,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelMessage,
        ModelRequest, ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::{Semaphore, mpsc};

struct Round {
    text: &'static str,
    gate: Option<Arc<Semaphore>>,
    partial: bool,
}
struct ScriptedModel {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<Round>>,
    seen: mpsc::UnboundedSender<ModelRequest>,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        _context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        let round = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted response must be declared");
        self.seen.send(request).unwrap();
        Box::pin(async move {
            let mut initial = Vec::new();
            if round.partial {
                initial.push(Ok(ModelEvent::text_delta(round.text)));
            }
            let tail = stream::once(async move {
                if let Some(gate) = round.gate {
                    gate.acquire_owned().await.unwrap().forget();
                }
                let mut events = Vec::new();
                if !round.partial {
                    events.push(Ok(ModelEvent::text_delta(round.text)));
                }
                events.push(Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }));
                stream::iter(events)
            })
            .flatten();
            Ok(Box::pin(stream::iter(initial).chain(tail)) as ModelStream)
        })
    }
}
struct Resources {
    model: Arc<dyn Model>,
}
impl ResourceResolver for Resources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        if config.model.as_str() == "test/model" {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "test model is not registered",
            ))
        }
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            if config.model.as_str() != "test/model" {
                return Err(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "test model is not registered",
                ));
            }
            let prompt = Text::new(format!("framework\n{}", config.role.instructions));
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    Arc::clone(&self.model),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(Some(prompt))),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
fn setup(
    rounds: Vec<Round>,
) -> (
    Arc<Host>,
    ConnectionHandle,
    SessionRef,
    mpsc::UnboundedReceiver<ModelRequest>,
) {
    let (seen, received) = mpsc::unbounded_channel();
    let model = ScriptedModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128_000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(rounds.into()),
        seen,
    };
    let host = Host::memory(Arc::new(Resources {
        model: Arc::new(model),
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "test/model".parse().unwrap();
    config.role.instructions = "old role".into();
    let receipt = host
        .create_agent(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "test Agent".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .unwrap();
    receipt.validate_call_return().unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    (host, connection, target, received)
}
fn input(mode: MessageMode, text: &str) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode,
        text: text.into(),
        in_reply_to: None,
    }
}
fn send(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    mode: MessageMode,
    text: &str,
) -> MessageAccepted {
    host.send(caller, SendMessage::new(input(mode, text), target.clone()))
        .unwrap()
}
fn final_round(text: &'static str) -> Round {
    Round {
        text,
        gate: None,
        partial: false,
    }
}
async fn next_request(rx: &mut mpsc::UnboundedReceiver<ModelRequest>) -> ModelRequest {
    tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap()
}
fn user_texts(request: &ModelRequest) -> Vec<String> {
    request
        .messages()
        .iter()
        .filter_map(|message| match message {
            ModelMessage::User(text) => {
                let (context, original) = input_context::decode(text);
                assert_eq!(context.source, MessageSource::User);
                assert!(context.in_reply_to.is_none());
                Some(original.to_owned())
            }
            _ => None,
        })
        .collect()
}
fn system(request: &ModelRequest) -> String {
    request
        .messages()
        .iter()
        .find_map(|message| match message {
            ModelMessage::System(text) => Some(text.to_string()),
            _ => None,
        })
        .unwrap()
}
async fn settled(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    count: usize,
) -> Vec<TurnInfo> {
    let mut states = host.session_states(caller, target).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let turns = host.turns(caller, target).unwrap();
            if turns.len() == count && turns.iter().all(|turn| turn.state == TurnState::Finished) {
                return turns;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .expect("owner did not settle declared turns")
}

async fn wait_state(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    ready: impl Fn(&minicore_host_core::SessionStateData) -> bool,
) -> minicore_host_core::SessionStateData {
    let mut states = host.session_states(caller, target).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let state = states.snapshot().unwrap().data;
            if ready(&state) {
                return state;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .expect("owner did not publish the expected state")
}

async fn wait_reclaimed(host: &Host, caller: &CallerContext, target: &SessionRef) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match host.session_state(caller, target) {
                Ok(snapshot) => {
                    if snapshot.data.lifecycle == SessionLifecycle::Closed {
                        assert!(
                            !snapshot.data.preparing
                                && snapshot.data.active_turn_id.is_none()
                                && !snapshot.data.saving
                        );
                    }
                }
                Err(fault) => {
                    assert!(matches!(
                        fault.kind(),
                        Some(FaultKind::AgentSessionNotFound | FaultKind::HostAgentNotFound)
                    ));
                    return;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("closed memory Session was not reclaimed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initial_batch_uses_two_independent_queues_once() {
    let (host, connection, target, mut seen) = setup(vec![final_round("one"), final_round("two")]);
    let caller = connection.caller();
    host.pause(&caller, &target).unwrap();
    let f1 = send(&host, &caller, &target, MessageMode::FollowUp, "F1");
    let s1 = send(&host, &caller, &target, MessageMode::Steer, "S1");
    let s2 = send(&host, &caller, &target, MessageMode::Steer, "S2");
    let f2 = send(&host, &caller, &target, MessageMode::FollowUp, "F2");
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    host.resume(&caller, &target).unwrap();
    let first = next_request(&mut seen).await;
    assert_eq!(user_texts(&first), ["S1", "S2", "F1"]);
    let second = next_request(&mut seen).await;
    assert_eq!(user_texts(&second), ["S1", "S2", "F1", "F2"]);
    let turns = settled(&host, &caller, &target, 2).await;
    for receipt in [s1, s2, f1] {
        assert!(
            matches!(host.message(&caller,&target,&receipt.message_id).unwrap().binding,MessageBinding::Bound{turn_id} if turn_id == turns[0].id)
        );
    }
    assert!(
        matches!(host.message(&caller,&target,&f2.message_id).unwrap().binding,MessageBinding::Bound{turn_id} if turn_id == turns[1].id)
    );
    assert_eq!(
        host.history(&caller, &target)
            .unwrap()
            .iter()
            .filter(|item| matches!(item, HistoryItem::User(_)))
            .count(),
        4
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn steer_before_final_claim_continues_same_turn_but_follow_up_does_not() {
    let gate = Arc::new(Semaphore::new(0));
    let (host, connection, target, mut seen) = setup(vec![
        Round {
            text: "first",
            gate: Some(gate.clone()),
            partial: false,
        },
        final_round("second"),
        final_round("next"),
    ]);
    let caller = connection.caller();
    send(&host, &caller, &target, MessageMode::FollowUp, "root");
    next_request(&mut seen).await;
    let before = host
        .session_state(&caller, &target)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    let follow = send(&host, &caller, &target, MessageMode::FollowUp, "later");
    let steer = send(&host, &caller, &target, MessageMode::Steer, "continue");
    gate.add_permits(1);
    assert_eq!(
        user_texts(&next_request(&mut seen).await),
        ["root", "continue"]
    );
    assert_eq!(
        user_texts(&next_request(&mut seen).await),
        ["root", "continue", "later"]
    );
    let turns = settled(&host, &caller, &target, 2).await;
    assert_eq!(turns[0].id, before);
    assert_eq!(turns[0].requests, 2);
    assert_eq!(turns[1].requests, 1);
    assert!(
        matches!(host.message(&caller,&target,&steer.message_id).unwrap().binding,MessageBinding::Bound{turn_id} if turn_id==before)
    );
    assert!(
        matches!(host.message(&caller,&target,&follow.message_id).unwrap().binding,MessageBinding::Bound{turn_id} if turn_id==turns[1].id)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn steer_after_final_claim_creates_new_turn_without_lost_wake() {
    let second_gate = Arc::new(Semaphore::new(0));
    let (host, connection, target, mut seen) = setup(vec![
        final_round("finished first"),
        Round {
            text: "finished second",
            gate: Some(second_gate.clone()),
            partial: false,
        },
    ]);
    let caller = connection.caller();
    let first = send(&host, &caller, &target, MessageMode::FollowUp, "first");
    next_request(&mut seen).await;
    // Observing the actual finished fact proves the final-claim boundary won.
    // It does not assume the driver has already exited its scheduling loop.
    let first_turn = settled(&host, &caller, &target, 1).await.remove(0);
    let later = send(&host, &caller, &target, MessageMode::Steer, "after final");
    assert_eq!(
        user_texts(&next_request(&mut seen).await),
        ["first", "after final"]
    );
    let active = host
        .session_state(&caller, &target)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    assert_ne!(active, first_turn.id);
    assert!(
        matches!(host.message(&caller, &target, &first.message_id).unwrap().binding,
        MessageBinding::Bound { turn_id } if turn_id == first_turn.id)
    );
    assert!(
        matches!(host.message(&caller, &target, &later.message_id).unwrap().binding,
        MessageBinding::Bound { turn_id } if turn_id == active)
    );
    second_gate.add_permits(1);
    let turns = settled(&host, &caller, &target, 2).await;
    assert_eq!(turns[0].id, first_turn.id);
    assert_eq!(turns[1].id, active);
    assert!(
        turns
            .iter()
            .all(|turn| turn.requests == 1 && turn.outcome == Some(TurnOutcome::Completed))
    );
}

// The single-thread executor keeps the synchronous cancel/send/repeat sequence
// inside one controlled scheduling interval before the Runtime can finish.
#[tokio::test]
async fn cancellation_keeps_later_messages_and_does_not_pause() {
    let gate = Arc::new(Semaphore::new(0));
    let (host, connection, target, mut seen) = setup(vec![
        Round {
            text: "partial",
            gate: Some(gate),
            partial: true,
        },
        final_round("next"),
    ]);
    let caller = connection.caller();
    send(&host, &caller, &target, MessageMode::FollowUp, "first");
    next_request(&mut seen).await;
    // Yield until the model stream can publish its prefix; cancellation itself
    // does not require partial to exist, only that any actual prefix is retained.
    tokio::task::yield_now().await;
    let id = host
        .session_state(&caller, &target)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    send(&host, &caller, &target, MessageMode::FollowUp, "remove me");
    let cancelled = host.cancel_turn(&caller, &target, &id, false).unwrap();
    assert_eq!(cancelled.status, CancelStatus::Claimed);
    assert_eq!(cancelled.cleared.len(), 1);
    let later = send(
        &host,
        &caller,
        &target,
        MessageMode::Steer,
        "arrived after cancel",
    );
    assert_eq!(later.result.queue, InputQueue::NextTurn);
    let repeated = host.cancel_turn(&caller, &target, &id, false).unwrap();
    assert_ne!(repeated.status, CancelStatus::Claimed);
    assert!(repeated.cleared.is_empty());
    next_request(&mut seen).await;
    let turns = settled(&host, &caller, &target, 2).await;
    assert_eq!(turns[0].outcome, Some(TurnOutcome::Cancelled));
    assert_eq!(turns[1].outcome, Some(TurnOutcome::Completed));
    assert_eq!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .scheduling,
        Scheduling::Auto
    );
    let stale = host.cancel_turn(&caller, &target, &id, false).unwrap();
    assert_eq!(stale.status, CancelStatus::NotActive);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_lifetime_not_a_caller_clone_controls_operation_rights() {
    let (host, operator, target, _) = setup(Vec::new());
    let old = operator.caller();
    let observer = host.connect().unwrap();
    assert!(!observer.initial_info().can_operate);
    assert_eq!(
        host.pause(&observer.caller(), &target).unwrap_err().kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    operator.close();
    assert_eq!(
        host.pause(&old, &target).unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert_eq!(
        host.pause(&observer.caller(), &target).unwrap_err().kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    let new = host.connect().unwrap();
    assert!(new.initial_info().can_operate);
    operator.close();
    host.pause(&new.caller(), &target).unwrap();
    assert!(host.info(&observer.caller()).unwrap().control.occupied);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_execution_outlives_its_connection() {
    let gate = Arc::new(Semaphore::new(0));
    let (host, connection, target, mut seen) = setup(vec![Round {
        text: "done",
        gate: Some(gate.clone()),
        partial: false,
    }]);
    send(
        &host,
        &connection.caller(),
        &target,
        MessageMode::FollowUp,
        "work",
    );
    next_request(&mut seen).await;
    drop(connection);
    let new = host.connect().unwrap();
    gate.add_permits(1);
    let turns = settled(&host, &new.caller(), &target, 1).await;
    assert_eq!(turns[0].outcome, Some(TurnOutcome::Completed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configuration_stays_bound_across_internal_loops() {
    let gate = Arc::new(Semaphore::new(0));
    let (host, connection, target, mut seen) = setup(vec![
        Round {
            text: "first",
            gate: Some(gate.clone()),
            partial: false,
        },
        final_round("continue"),
        final_round("new"),
    ]);
    let caller = connection.caller();
    send(&host, &caller, &target, MessageMode::FollowUp, "root");
    assert_eq!(
        system(&next_request(&mut seen).await),
        "framework\nold role"
    );
    host.update_session(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            role: Some(Role {
                name: "new".into(),
                description: String::new(),
                instructions: "new role".into(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    send(&host, &caller, &target, MessageMode::Steer, "same turn");
    send(&host, &caller, &target, MessageMode::FollowUp, "new turn");
    gate.add_permits(1);
    assert_eq!(
        system(&next_request(&mut seen).await),
        "framework\nold role"
    );
    assert_eq!(
        system(&next_request(&mut seen).await),
        "framework\nnew role"
    );
    let turns = settled(&host, &caller, &target, 2).await;
    assert_eq!(turns[0].config_revision, 0);
    assert_eq!(turns[1].config_revision, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_session_does_not_block_another_session_or_receipts() {
    let gate = Arc::new(Semaphore::new(0));
    let (host, connection, first, mut seen) = setup(vec![
        Round {
            text: "slow",
            gate: Some(gate.clone()),
            partial: false,
        },
        final_round("fast"),
    ]);
    let caller = connection.caller();
    send(&host, &caller, &first, MessageMode::FollowUp, "slow");
    next_request(&mut seen).await;
    let info = host.info(&caller).unwrap();
    let created = host
        .create_session(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: AgentRef {
                    host_id: info.host_id,
                    agent_id: first.agent_id.clone(),
                },
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .unwrap();
    let second = SessionRef {
        agent_id: first.agent_id.clone(),
        session_id: created.session_id.unwrap(),
    };
    send(&host, &caller, &second, MessageMode::FollowUp, "fast");
    next_request(&mut seen).await;
    assert_eq!(
        settled(&host, &caller, &second, 1).await[0].outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(
        host.session_state(&caller, &first)
            .unwrap()
            .data
            .active_turn_id
            .is_some()
    );
    gate.add_permits(1);
    settled(&host, &caller, &first, 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn close_clears_default_and_unload_preserves_memory_history() {
    let (host, connection, target, mut seen) = setup(vec![final_round("one"), final_round("two")]);
    let caller = connection.caller();
    send(&host, &caller, &target, MessageMode::FollowUp, "one");
    next_request(&mut seen).await;
    settled(&host, &caller, &target, 1).await;
    host.unload(&caller, &target).unwrap();
    send(&host, &caller, &target, MessageMode::FollowUp, "two");
    assert_eq!(user_texts(&next_request(&mut seen).await), ["one", "two"]);
    settled(&host, &caller, &target, 2).await;
    let agent_ref = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id.clone(),
    };
    // Keep another child open so the default check does not depend on retaining
    // an otherwise empty memory Agent. Closing never selects this child instead.
    let remaining = host
        .create_session(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent_ref.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .unwrap();
    host.close_session(&caller, &target).unwrap();
    wait_reclaimed(&host, &caller, &target).await;
    assert!(
        host.agent_info(&caller, &agent_ref)
            .unwrap()
            .default_session_id
            .is_none()
    );
    assert_eq!(
        host.send(
            &caller,
            SendMessage::new(input(MessageMode::FollowUp, "no"), agent_ref)
        )
        .unwrap_err()
        .kind(),
        Some(FaultKind::AgentDefaultSessionUnavailable)
    );
    assert_eq!(
        host.resume(&caller, &target).unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    assert_eq!(
        host.session_state(
            &caller,
            &SessionRef {
                agent_id: target.agent_id,
                session_id: remaining.session_id.unwrap(),
            }
        )
        .unwrap()
        .data
        .lifecycle,
        SessionLifecycle::Open
    );
}

struct GatedResources {
    inner: Resources,
    started: mpsc::UnboundedSender<()>,
    gate: Arc<Semaphore>,
    resolved: mpsc::UnboundedSender<()>,
}
impl ResourceResolver for GatedResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.started.send(()).unwrap();
            self.gate.acquire().await.unwrap().forget();
            let result = self.inner.resolve(config).await;
            self.resolved.send(()).unwrap();
            result
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_close_during_preparation_must_finish_after_resolution() {
    let (seen, _received) = mpsc::unbounded_channel();
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let (resolved_tx, mut resolved_rx) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let model = ScriptedModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128_000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(VecDeque::new()),
        seen,
    };
    let host = Host::memory(Arc::new(GatedResources {
        inner: Resources {
            model: Arc::new(model),
        },
        started: started_tx,
        gate: gate.clone(),
        resolved: resolved_tx,
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "test/model".parse().unwrap();
    let created = host
        .create_agent(
            &caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "test Agent".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "prepare this",
    );
    started_rx.recv().await.unwrap();
    assert!(host.session_state(&caller, &target).unwrap().data.preparing);
    host.close_session(&caller, &target).unwrap();
    let closing = host.session_state(&caller, &target).unwrap().data;
    assert_eq!(closing.lifecycle, SessionLifecycle::Closing);
    assert!(closing.preparing);
    gate.add_permits(1);
    resolved_rx.recv().await.unwrap();
    wait_reclaimed(&host, &caller, &target).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_closed_connection_must_not_keep_receiving_session_updates() {
    let (host, operator, target, _seen) = setup(Vec::new());
    let observer = host.connect().unwrap();
    let mut subscription = host.session_states(&observer.caller(), &target).unwrap();
    subscription.snapshot().unwrap();
    observer.close();
    host.pause(&operator.caller(), &target).unwrap();
    assert!(
        subscription.changed().await.is_err(),
        "invalid connection still received fresh session state: {:?}",
        subscription.snapshot()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_idle_agent_stop_must_reach_stopped() {
    let (host, connection, target, _seen) = setup(Vec::new());
    let caller = connection.caller();
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id,
    };
    host.stop_agent(&caller, &agent).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while host.agent_info(&caller, &agent).unwrap().lifecycle != AgentLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Agent stop owner did not settle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_session_update_must_reject_missing_resource_before_commit() {
    let (host, connection, target, _seen) = setup(Vec::new());
    let caller = connection.caller();
    let old = host.session_info(&caller, &target).unwrap();
    let result = host.update_session(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            model: Some("missing/model".parse().unwrap()),
            ..Default::default()
        },
    );
    assert!(
        result.is_err(),
        "invalid resource reference was committed: {:?}",
        result
    );
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config,
        old.config
    );
}

struct FailOnceModel {
    descriptor: ModelDescriptor,
    calls: std::sync::atomic::AtomicUsize,
    seen: mpsc::UnboundedSender<ModelRequest>,
}
impl Model for FailOnceModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        _context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        self.seen.send(request).unwrap();
        let first = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        Box::pin(async move {
            let mut events = vec![Ok(ModelEvent::text_delta("partial actual output"))];
            if !first {
                events.push(Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }));
            }
            Ok(Box::pin(stream::iter(events)) as ModelStream)
        })
    }
}
fn setup_with_resources(
    resources: Arc<dyn ResourceResolver>,
) -> (Arc<Host>, ConnectionHandle, SessionRef) {
    let host = Host::memory(resources).unwrap();
    let connection = host.connect().unwrap();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "test/model".parse().unwrap();
    let receipt = host
        .create_agent(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "test Agent".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    (host, connection, target)
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_failed_model_must_pause_before_consuming_next_turn() {
    let (seen, mut received) = mpsc::unbounded_channel();
    let model = Arc::new(FailOnceModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128_000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        calls: std::sync::atomic::AtomicUsize::new(0),
        seen,
    });
    let (host, connection, target) = setup_with_resources(Arc::new(Resources {
        model: model.clone(),
    }));
    let caller = connection.caller();
    host.pause(&caller, &target).unwrap();
    send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "first failing task",
    );
    send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "must wait for resume",
    );
    host.resume(&caller, &target).unwrap();
    next_request(&mut received).await;
    settled(&host, &caller, &target, 1).await;
    assert_eq!(
        model.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "failed Turn consumed queued follow-up without resume"
    );
    assert_eq!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .scheduling,
        Scheduling::Paused
    );
}

struct PanickingResources;
impl ResourceResolver for PanickingResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move { panic!("intentional test resolver panic") })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_preparation_panic_must_be_reported_and_close_settle() {
    let (host, connection, target) = setup_with_resources(Arc::new(PanickingResources));
    let caller = connection.caller();
    send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "accepted task",
    );
    let snapshot = wait_state(&host, &caller, &target, |state| {
        state.block_reason.is_some() && !state.preparing
    })
    .await;
    assert!(
        !snapshot.preparing,
        "dead owner still claims preparation is running"
    );
    assert!(
        snapshot.block_reason.is_some(),
        "owner panic is not observable in authoritative state"
    );
    host.close_session(&caller, &target).unwrap();
    wait_reclaimed(&host, &caller, &target).await;
}

#[test]
fn review_send_outside_tokio_must_not_panic_after_acceptance() {
    let (host, connection, target, _seen) = setup(Vec::new());
    let caller = connection.caller();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host.send(
            &caller,
            SendMessage::new(
                input(MessageMode::FollowUp, "accepted from a synchronous caller"),
                target.clone(),
            ),
        )
    }));
    assert!(
        result.is_ok(),
        "send panicked after accepting input instead of returning a typed result"
    );
    assert!(
        host.session_state(&caller, &target).is_ok(),
        "session state mutex became poisoned"
    );
}

#[test]
fn review_memory_default_close_orders_remain_authoritative() {
    let (host, connection, first, _) = setup(Vec::new());
    let caller = connection.caller();
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: first.agent_id.clone(),
    };
    let make_child = || {
        let receipt = host
            .create_session(
                &caller,
                SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: agent.clone(),
                    mode: None,
                    overrides: None,
                    initial_message: None,
                },
            )
            .unwrap();
        SessionRef {
            agent_id: first.agent_id.clone(),
            session_id: receipt.session_id.unwrap(),
        }
    };
    let set_first = make_child();
    host.set_default_session(&caller, &agent, Some(set_first.session_id.clone()))
        .unwrap();
    host.close_session(&caller, &set_first).unwrap();
    assert!(
        host.agent_info(&caller, &agent)
            .unwrap()
            .default_session_id
            .is_none()
    );
    assert_eq!(
        host.set_default_session(&caller, &agent, Some(set_first.session_id))
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentSessionNotFound)
    );

    host.set_default_session(&caller, &agent, Some(first.session_id.clone()))
        .unwrap();
    let close_first = make_child();
    host.close_session(&caller, &close_first).unwrap();
    assert_eq!(
        host.set_default_session(&caller, &agent, Some(close_first.session_id))
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    assert_eq!(
        host.agent_info(&caller, &agent).unwrap().default_session_id,
        Some(first.session_id)
    );
}

#[test]
fn review_memory_default_close_race_preserves_actual_acceptance_order() {
    for _ in 0..128 {
        let (host, connection, first, _) = setup(Vec::new());
        let caller = connection.caller();
        let agent = AgentRef {
            host_id: host.info(&caller).unwrap().host_id,
            agent_id: first.agent_id.clone(),
        };
        let created = host
            .create_session(
                &caller,
                SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: agent.clone(),
                    mode: None,
                    overrides: None,
                    initial_message: None,
                },
            )
            .unwrap();
        let target = SessionRef {
            agent_id: first.agent_id,
            session_id: created.session_id.unwrap(),
        };
        let start = std::sync::Barrier::new(3);
        let set_result = std::thread::scope(|scope| {
            let set = scope.spawn(|| {
                start.wait();
                host.set_default_session(&caller, &agent, Some(target.session_id.clone()))
            });
            let close = scope.spawn(|| {
                start.wait();
                host.close_session(&caller, &target)
            });
            start.wait();
            let result = set.join().unwrap();
            close.join().unwrap().unwrap();
            result
        });
        let expected = if set_result.is_ok() {
            None
        } else {
            assert!(matches!(
                set_result.unwrap_err().kind(),
                Some(FaultKind::AgentDefaultSessionUnavailable | FaultKind::AgentSessionNotFound)
            ));
            Some(first.session_id)
        };
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().default_session_id,
            expected
        );
        assert_eq!(
            host.session_state(&caller, &target).unwrap_err().kind(),
            Some(FaultKind::AgentSessionNotFound)
        );
    }
}

#[test]
fn review_memory_default_revocation_does_not_revoke_accepted_facts() {
    let (host, operator, first, _) = setup(Vec::new());
    let caller = operator.caller();
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: first.agent_id.clone(),
    };
    let observer = host.connect().unwrap();
    let created = host
        .create_session(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .unwrap();
    let selected = created.session_id.unwrap();
    assert_eq!(
        host.set_default_session(&observer.caller(), &agent, Some(selected.clone()))
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    host.set_default_session(&caller, &agent, Some(selected.clone()))
        .unwrap();
    operator.close();
    assert_eq!(
        host.set_default_session(&caller, &agent, Some(first.session_id))
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert_eq!(
        host.set_default_session(&observer.caller(), &agent, None)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    assert_eq!(
        host.agent_info(&observer.caller(), &agent)
            .unwrap()
            .default_session_id,
        Some(selected)
    );
    let replacement = host.connect().unwrap();
    host.set_default_session(&replacement.caller(), &agent, None)
        .unwrap();
    assert!(
        host.agent_info(&replacement.caller(), &agent)
            .unwrap()
            .default_session_id
            .is_none()
    );
}

fn control_request() -> SessionControlRequest {
    SessionControlRequest {
        operation_id: OperationId::new().unwrap(),
    }
}
fn assert_receipt(host: &Host, caller: &CallerContext, expected: Receipt) {
    assert_eq!(
        host.receipt_get(caller, expected.operation_id()).unwrap(),
        expected
    );
    let wire = serde_json::to_value(&expected).unwrap();
    assert_eq!(serde_json::from_value::<Receipt>(wire).unwrap(), expected);
}

#[tokio::test]
async fn canonical_session_controls_freeze_actual_acceptance_facts() {
    let (host, connection, target, _) = setup(Vec::new());
    let caller = connection.caller();
    let paused = host
        .session_pause(&caller, &target, control_request())
        .unwrap();
    assert_eq!(paused.result.scheduling, Scheduling::Paused);
    assert_eq!(paused.result.pause_reasons, [PauseReason::Explicit]);
    let loaded = host
        .session_load(&caller, &target, control_request())
        .unwrap();
    assert_eq!(loaded.result.residency, Residency::Loading);
    assert!(loaded.result.load_id.is_none());
    assert_eq!(loaded.result.scheduling, Scheduling::Paused);
    wait_state(&host, &caller, &target, |state| {
        state.residency == Residency::Loaded
    })
    .await;
    assert_receipt(&host, &caller, Receipt::SessionControl(loaded));
    let resumed = host
        .session_resume(&caller, &target, control_request())
        .unwrap();
    assert_eq!(resumed.result.scheduling, Scheduling::Auto);
    assert_eq!(resumed.result.residency, Residency::Loaded);
    assert!(resumed.result.load_id.is_some());
    let paused_again = host
        .session_pause(&caller, &target, control_request())
        .unwrap();
    let queued = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "keep while unloaded",
    );
    let unloaded = host
        .session_unload(&caller, &target, control_request())
        .unwrap();
    assert_eq!(unloaded.result.residency, Residency::Unloaded);
    assert!(unloaded.result.load_id.is_none());
    assert_eq!(
        unloaded.result.pause_reasons,
        [PauseReason::Explicit, PauseReason::UnloadedWithInput]
    );
    let state = host.session_state(&caller, &target).unwrap().data;
    assert_eq!(state.pause_reasons, unloaded.result.pause_reasons);
    assert_eq!(
        state.controller_revision,
        unloaded.result.controller_revision
    );
    assert_eq!(state.next_turn[0].message_id, queued.message_id);
    for receipt in [paused, resumed, paused_again, unloaded] {
        assert_receipt(&host, &caller, Receipt::SessionControl(receipt));
    }
}

#[tokio::test]
async fn canonical_reused_operation_ids_keep_independent_owner_facts() {
    let (host, connection, target, _) = setup(Vec::new());
    let caller = connection.caller();
    let request = control_request();
    let first = host
        .session_pause(&caller, &target, request.clone())
        .unwrap();
    let second = host
        .session_resume(&caller, &target, request.clone())
        .unwrap();
    let third = host
        .session_pause(&caller, &target, request.clone())
        .unwrap();
    assert!(third.result.controller_revision > first.result.controller_revision);
    assert_eq!(
        host.receipts(&caller, &request.operation_id).unwrap(),
        vec![
            Receipt::SessionControl(first),
            Receipt::SessionControl(second),
            Receipt::SessionControl(third),
        ]
    );
    assert_eq!(
        host.receipt_get(&caller, &request.operation_id)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );
    let agent_ref = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id.clone(),
    };
    let stop = host
        .agent_stop(
            &caller,
            &agent_ref,
            AgentStopRequest {
                operation_id: request.operation_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(stop.result.lifecycle, AgentStopLifecycle::Stopping);
    assert_eq!(
        host.receipts(&caller, &request.operation_id).unwrap().len(),
        4
    );
    assert_eq!(
        host.receipt_get(&caller, &request.operation_id)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );
}

#[tokio::test]
async fn canonical_turn_cancellation_never_retargets_or_clears_later_input() {
    let (host, connection, target, mut seen) = setup(vec![
        Round {
            text: "partial",
            gate: Some(Arc::new(Semaphore::new(0))),
            partial: true,
        },
        Round {
            text: "second",
            gate: Some(Arc::new(Semaphore::new(0))),
            partial: true,
        },
    ]);
    let caller = connection.caller();
    send(&host, &caller, &target, MessageMode::FollowUp, "first");
    next_request(&mut seen).await;
    let first_id = host
        .session_state(&caller, &target)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    let removed = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "remove at claim",
    );
    let cancel = host
        .turn_cancel(
            &caller,
            &target,
            TurnCancelRequest {
                operation_id: OperationId::new().unwrap(),
                turn_id: first_id.clone(),
                keep_inbox: false,
            },
        )
        .unwrap();
    assert!(cancel.result.cancel_claimed);
    assert_eq!(cancel.result.turn_state, TurnState::Cancelling);
    assert!(matches!(
        host.message(&caller, &target, &removed.message_id)
            .unwrap()
            .binding,
        MessageBinding::Removed
    ));
    let later = send(
        &host,
        &caller,
        &target,
        MessageMode::Steer,
        "keep after claim",
    );
    let repeated = host
        .turn_cancel(
            &caller,
            &target,
            TurnCancelRequest {
                operation_id: OperationId::new().unwrap(),
                turn_id: first_id.clone(),
                keep_inbox: false,
            },
        )
        .unwrap();
    assert!(!repeated.result.cancel_claimed);
    assert_eq!(repeated.result.turn_state, TurnState::Cancelling);
    assert!(matches!(
        host.message(&caller, &target, &later.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    next_request(&mut seen).await;
    let second_id = host
        .session_state(&caller, &target)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    assert_ne!(first_id, second_id);
    let next = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "do not clear for stale T1",
    );
    let stale = host
        .turn_cancel(
            &caller,
            &target,
            TurnCancelRequest {
                operation_id: OperationId::new().unwrap(),
                turn_id: first_id,
                keep_inbox: false,
            },
        )
        .unwrap();
    assert_eq!(stale.result.turn_state, TurnState::Finished);
    assert!(!stale.result.cancel_claimed);
    assert_eq!(
        host.turn_info(&caller, &target, &second_id).unwrap().state,
        TurnState::Running
    );
    assert!(matches!(
        host.message(&caller, &target, &next.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    let missing = host
        .turn_cancel(
            &caller,
            &target,
            TurnCancelRequest {
                operation_id: OperationId::new().unwrap(),
                turn_id: TurnId::new().unwrap(),
                keep_inbox: false,
            },
        )
        .unwrap_err();
    assert_eq!(missing.kind(), Some(FaultKind::SessionTurnNotFound));
    for receipt in [cancel, repeated, stale] {
        assert_receipt(&host, &caller, Receipt::TurnCancel(receipt));
    }
    host.session_cancel(
        &caller,
        &target,
        SessionCancelRequest {
            operation_id: OperationId::new().unwrap(),
            keep_inbox: false,
        },
    )
    .unwrap();
    settled(&host, &caller, &target, 2).await;
}

#[tokio::test]
async fn canonical_session_cancel_reports_idle_and_exact_current_turn() {
    let (host, connection, target, mut seen) = setup(vec![Round {
        text: "partial",
        gate: Some(Arc::new(Semaphore::new(0))),
        partial: true,
    }]);
    let caller = connection.caller();
    host.pause(&caller, &target).unwrap();
    let queued = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "idle queued",
    );
    let kept = host
        .session_cancel(
            &caller,
            &target,
            SessionCancelRequest {
                operation_id: OperationId::new().unwrap(),
                keep_inbox: true,
            },
        )
        .unwrap();
    assert!(kept.turn_id.is_none());
    assert!(kept.result.turn_state.is_none());
    assert!(!kept.result.cancel_claimed);
    assert!(matches!(
        host.message(&caller, &target, &queued.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    let cleared = host
        .session_cancel(
            &caller,
            &target,
            SessionCancelRequest {
                operation_id: OperationId::new().unwrap(),
                keep_inbox: false,
            },
        )
        .unwrap();
    assert!(cleared.turn_id.is_none());
    assert!(matches!(
        host.message(&caller, &target, &queued.message_id)
            .unwrap()
            .binding,
        MessageBinding::Removed
    ));
    send(&host, &caller, &target, MessageMode::FollowUp, "run now");
    host.resume(&caller, &target).unwrap();
    next_request(&mut seen).await;
    let current = host
        .session_state(&caller, &target)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    let cancelled = host
        .session_cancel(
            &caller,
            &target,
            SessionCancelRequest {
                operation_id: OperationId::new().unwrap(),
                keep_inbox: false,
            },
        )
        .unwrap();
    assert_eq!(cancelled.turn_id, Some(current));
    assert!(cancelled.result.cancel_claimed);
    assert_eq!(cancelled.result.turn_state, Some(TurnState::Cancelling));
    assert_eq!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .scheduling,
        Scheduling::Auto
    );
    for receipt in [kept, cleared, cancelled] {
        assert_receipt(&host, &caller, Receipt::SessionCancel(receipt));
    }
    settled(&host, &caller, &target, 1).await;
}

#[tokio::test]
async fn canonical_close_returns_accepted_facts_even_when_memory_is_reclaimed() {
    let (host, connection, target, _) = setup(Vec::new());
    let caller = connection.caller();
    let receipt = host
        .session_close(
            &caller,
            &target,
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(receipt.result.lifecycle, SessionCloseLifecycle::Closed);
    assert_eq!(receipt.session_id, target.session_id);
    assert!(host.session_info(&caller, &target).is_err());
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptNotFound)
    );
    assert!(host.agents(&caller).unwrap().is_empty());
}

#[tokio::test]
async fn canonical_agent_stop_preserves_queues_and_formed_lifecycle() {
    let (host, connection, target, mut seen) = setup(vec![Round {
        text: "partial",
        gate: Some(Arc::new(Semaphore::new(0))),
        partial: true,
    }]);
    let caller = connection.caller();
    send(&host, &caller, &target, MessageMode::FollowUp, "running");
    next_request(&mut seen).await;
    host.pause(&caller, &target).unwrap();
    let queued = send(&host, &caller, &target, MessageMode::FollowUp, "retained");
    let agent_ref = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id.clone(),
    };
    let first = host
        .agent_stop(
            &caller,
            &agent_ref,
            AgentStopRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(first.result.lifecycle, AgentStopLifecycle::Stopping);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while host.agent_info(&caller, &agent_ref).unwrap().lifecycle != AgentLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let again = host
        .agent_stop(
            &caller,
            &agent_ref,
            AgentStopRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(again.result.lifecycle, AgentStopLifecycle::Stopped);
    assert!(matches!(
        host.message(&caller, &target, &queued.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    host.start_agent(&caller, &agent_ref).unwrap();
    assert_eq!(
        host.agent_info(&caller, &agent_ref).unwrap().lifecycle,
        AgentLifecycle::Active
    );
    for receipt in [first, again] {
        assert_receipt(&host, &caller, Receipt::AgentStop(receipt));
    }
}

#[tokio::test]
async fn canonical_receipts_require_live_read_authority_and_current_write_authority() {
    let (host, operator, target, _) = setup(Vec::new());
    let observer = host.connect().unwrap();
    let request = control_request();
    assert_eq!(
        host.session_pause(&observer.caller(), &target, request.clone())
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    assert!(
        host.receipts(&observer.caller(), &request.operation_id)
            .unwrap()
            .is_empty()
    );
    let old = operator.caller();
    operator.close();
    assert_eq!(
        host.session_pause(&old, &target, request.clone())
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert_eq!(
        host.receipts(&old, &request.operation_id)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    let new = host.connect().unwrap();
    let receipt = host.session_pause(&new.caller(), &target, request).unwrap();
    assert_receipt(&host, &observer.caller(), Receipt::SessionControl(receipt));
}

#[tokio::test]
async fn canonical_shutdown_receipts_keep_each_invocations_strategy() {
    let (host, connection, target, _) = setup(Vec::new());
    let caller = connection.caller();
    host.pause(&caller, &target).unwrap();
    send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "not executed while paused",
    );
    let operation = OperationId::new().unwrap();
    let first = host
        .shutdown(
            &caller,
            HostShutdownRequest {
                operation_id: operation.clone(),
                strategy: HostShutdownStrategy::Graceful,
            },
        )
        .unwrap();
    assert_eq!(first.result.lifecycle, HostLifecycle::Draining);
    let second = host
        .shutdown(
            &caller,
            HostShutdownRequest {
                operation_id: operation.clone(),
                strategy: HostShutdownStrategy::Force,
            },
        )
        .unwrap();
    assert_eq!(second.result.lifecycle, HostLifecycle::Stopping);
    assert_eq!(
        host.receipts(&caller, &operation).unwrap(),
        [Receipt::HostShutdown(first), Receipt::HostShutdown(second)]
    );
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap_err().kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while host.info(&caller).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(host.receipts(&caller, &operation).unwrap().len(), 2);
}

#[test]
fn canonical_receipt_lookup_distinguishes_absence_ambiguity_and_unavailable_method() {
    let (host, connection, target, _) = setup(Vec::new());
    let caller = connection.caller();
    let operation = OperationId::new().unwrap();
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id.clone(),
    };
    for missing in [
        host.receipt_get(&caller, &operation).unwrap_err(),
        host.receipt(&caller, &operation).unwrap_err(),
        host.session_creation_receipt(&caller, &agent, &operation)
            .unwrap_err(),
    ] {
        assert_eq!(missing.kind(), Some(FaultKind::HostReceiptNotFound));
        assert_eq!(missing.code(), 1202);
    }
    let request = SessionControlRequest {
        operation_id: operation.clone(),
    };
    let first = host
        .session_pause(&caller, &target, request.clone())
        .unwrap();
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::SessionControl(first)
    );
    host.session_pause(&caller, &target, request).unwrap();
    let ambiguous = host.receipt_get(&caller, &operation).unwrap_err();
    assert_eq!(ambiguous.kind(), Some(FaultKind::HostReceiptAmbiguous));
    assert_eq!(ambiguous.code(), 1203);
    assert_eq!(host.receipts(&caller, &operation).unwrap().len(), 2);
    let unsupported = require_implemented(Method::HostReceiptGet, &BTreeSet::new()).unwrap_err();
    assert_eq!(unsupported.kind(), Some(FaultKind::HostMethodUnavailable));
    assert_eq!(unsupported.code(), 1401);
}
