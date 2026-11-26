//! Direct owner operations and real Runtime/journal work, with deterministic
//! test-only gates. No discovery field is set directly to manufacture a fact.
use super::*;
use crate::{
    DelegationCompletion, DelegationRegistration, DelegationSink, OwnerFuture, ScopeHooks,
};
use minicore_host_runtime::{
    AssistantHistory, UserHistory, UserMessageKind,
    model::{AssistantPart, Usage},
};
use std::{sync::Condvar, time::Duration};

#[tokio::test]
async fn task071_owned_history_save_stays_visible_until_actual_acknowledgement() {
    let gate = Arc::new(Semaphore::new(0));
    let (entered, mut received) = mpsc::unbounded_channel();
    let fixture = setup(
        Arc::new(Resources {
            model: model(vec![final_events("saved result")]),
            tools: ToolSet::default(),
        }),
        |inner| {
            Arc::new(GatedJournal {
                inner,
                gates: Mutex::new(VecDeque::from([gate.clone()])),
                entered,
                count: AtomicUsize::new(0),
            })
        },
    );
    send(&fixture, "accepted real input");
    assert_eq!(received.recv().await, Some(0));
    let caller = fixture.connection.caller();
    let snapshot = fixture.session.snapshot(&caller).unwrap();
    assert!(snapshot.data.owned_work.history_save);
    assert!(
        fixture
            .session
            .summary(&caller)
            .unwrap()
            .unwrap()
            .owned_work
            .history_save
    );
    assert!(snapshot.data.end_reason.is_none());
    gate.add_permits(1);
    until(&fixture, |state| {
        state.active.is_none() && !state.completed.is_empty()
    })
    .await;
    assert!(
        !fixture
            .session
            .snapshot(&caller)
            .unwrap()
            .data
            .owned_work
            .history_save
    );
    assert!(
        records(&fixture)
            .iter()
            .any(|record| matches!(record.entry, HistoryEntry::TurnFinished { .. }))
    );
}

struct UtilityGate {
    descriptor: ModelDescriptor,
    entered: Semaphore,
    release: Semaphore,
}
impl Model for UtilityGate {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        Box::pin(async {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Ok(Box::pin(futures_util::stream::iter(
                final_events("Compact summary of prior work.")
                    .into_iter()
                    .map(Ok),
            )) as ModelStream)
        })
    }
}
#[tokio::test]
async fn task071_owned_compaction_reports_only_actual_active_operation() {
    let utility = Arc::new(UtilityGate {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            4096,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let fixture = setup(
        Arc::new(Resources {
            model: utility.clone(),
            tools: ToolSet::default(),
        }),
        |inner| inner,
    );
    let mut entries = Vec::new();
    for _ in 0..3 {
        let turn_id = TurnId::new().unwrap();
        let loop_id = LoopId::new().unwrap();
        entries.push(HistoryEntry::Input {
            received_at: minicore_host_protocol::Timestamp::now(),
            turn_id: turn_id.clone(),
            message_id: MessageId::new().unwrap(),
            source: MessageSource::User,
            mode: MessageMode::FollowUp,
            in_reply_to: None,
            user: UserHistory {
                loop_id,
                kind: UserMessageKind::Prompt,
                input: UserInput::text("prior task ".repeat(100)).unwrap(),
            },
        });
        entries.push(HistoryEntry::Runtime {
            turn_id: turn_id.clone(),
            response_index: 0,
            item: HistoryItem::Assistant(AssistantHistory {
                loop_id,
                request_index: 0,
                model: "test/model".parse().unwrap(),
                reasoning: ReasoningPreference::Auto,
                content: vec![AssistantPart::Text(
                    "prior response facts ".repeat(100).into(),
                )],
                provider_replay: None,
                finish_reason: ModelFinishReason::Stop,
                usage: Usage::default(),
            }),
        });
        entries.push(HistoryEntry::TurnFinished {
            finished_at: minicore_host_protocol::Timestamp::now(),
            requests: 1,
            tool_rounds: 0,
            model_attempts: vec![1],
            turn_id,
            outcome: TurnOutcome::Completed,
            reason: None,
        });
    }
    fixture
        .session
        .backend
        .append(test_history_records(&fixture.session.id, entries))
        .await
        .unwrap();
    fixture
        .session
        .control(
            &fixture.connection.caller(),
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            SessionControlKind::Load,
        )
        .unwrap();
    until(&fixture, |state| state.history_loaded && !state.preparing).await;
    let operation = OperationId::new().unwrap();
    fixture
        .session
        .compact(
            &fixture.connection.caller(),
            CompactRequest {
                operation_id: operation.clone(),
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), utility.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let snapshot = fixture
        .session
        .snapshot(&fixture.connection.caller())
        .unwrap();
    let work = snapshot.data.owned_work.compaction.unwrap();
    assert!(
        matches!(work.source, CompactionSource::Manual { operation_id } if operation_id == operation)
    );
    assert!(matches!(work.state, CompactionState::Summarizing {}));
    assert!(snapshot.data.active_turn_id.is_none());
    utility.release.add_permits(1);
    until(&fixture, |state| state.compaction.is_none()).await;
    let snapshot = fixture
        .session
        .snapshot(&fixture.connection.caller())
        .unwrap();
    assert!(
        snapshot.data.owned_work.compaction.is_none(),
        "finished retained diagnostics are not active work"
    );
    assert!(snapshot.data.compaction.unwrap().state.finished());
}

struct ScopeGate {
    entered: Semaphore,
    release: Semaphore,
}
impl ScopeHooks for ScopeGate {
    fn seal(&self) {}
    fn close_members(&self) -> OwnerFuture<'_, ()> {
        Box::pin(async {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Ok(())
        })
    }
    fn root_closed(&self) {}
}
struct HandoffGate {
    entered: Semaphore,
    released: Mutex<bool>,
    changed: Condvar,
    completion: Mutex<Option<DelegationCompletion>>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn settled_delegation_survives_delayed_acceptance_and_saves_typed_notice() {
    let call = "delayed-start";
    let mut tools = ToolSet::builder();
    tools.register(CountTool {
        spec: ToolSpec::new(
            "count".parse().unwrap(),
            "real owner fixture",
            serde_json::json!({}),
        )
        .unwrap(),
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let parent = setup(
        Arc::new(Resources {
            model: model(vec![
                vec![
                    ModelEvent::ToolCallStart {
                        tool_call_id: call.parse().unwrap(),
                        tool_name: "count".parse().unwrap(),
                    },
                    ModelEvent::tool_call_arguments_delta(call.parse().unwrap(), "{}"),
                    ModelEvent::ToolCallEnd {
                        tool_call_id: call.parse().unwrap(),
                    },
                    ModelEvent::Finish {
                        reason: ModelFinishReason::ToolCalls,
                    },
                ],
                final_events("startup returned"),
                final_events("notice consumed"),
            ]),
            tools: tools.build().unwrap(),
        }),
        |inner| inner,
    );
    send(&parent, "parent work");
    until(&parent, |state| state.completed.len() == 1).await;
    let caller = parent.connection.caller();
    let parent_ref = SessionRef {
        agent_id: parent.session.agent_id.clone(),
        session_id: parent.session.id.clone(),
    };
    let turn_id = parent.session.state.lock().unwrap().completion_order[0].clone();
    let origin = ToolCallRef {
        turn_id,
        tool_call_id: ToolCallId(call.into()),
    };
    let before = parent
        .session
        .tool_get(&caller, &origin.turn_id, &origin.tool_call_id)
        .unwrap()
        .1;
    assert_eq!(before.state.state, crate::ToolCallState::Finished);
    parent.session.pause(&caller).unwrap();
    let scope = parent
        .session
        .register_scope_owned(Arc::new(ScopeGate {
            entered: Semaphore::new(0),
            release: Semaphore::new(1),
        }))
        .unwrap();
    let child = SessionController::new(
        SessionId::new().unwrap(),
        parent.session.agent_id.clone(),
        parent.session.host_id.clone(),
        parent.session.incarnation.clone(),
        SessionConfig::default(),
        parent.access.clone(),
        Arc::new(Resources {
            model: model(vec![final_events("actual child result")]),
            tools: ToolSet::default(),
        }),
        Arc::new(MemoryHistory),
        vec![],
        Arc::new(AtomicBool::new(true)),
    );
    child.publish_initial();
    let input = MessageInput {
        message_id: MessageId::new().unwrap(),
        text: "child work".into(),
        mode: MessageMode::FollowUp,
        in_reply_to: None,
    };
    let registration = DelegationRegistration {
        host_id: parent.session.host_id.clone(),
        host_incarnation: parent.session.incarnation.clone(),
        root: parent_ref.clone(),
        parent: parent_ref.clone(),
        child: SessionRef {
            agent_id: child.agent_id.clone(),
            session_id: child.id.clone(),
        },
        task_message_id: input.message_id.clone(),
        origin: origin.clone(),
        temporary: false,
        name: None,
    };
    let sink = Arc::new(HandoffGate {
        entered: Semaphore::new(0),
        released: Mutex::new(false),
        changed: Condvar::new(),
        completion: Mutex::new(None),
    });
    let permit = child
        .register_delegation_owned(
            registration.clone(),
            Arc::downgrade(&parent.session),
            scope.state.clone(),
            sink.clone(),
        )
        .unwrap();
    let (_, record, delayed_acceptance, _) = child
        .receive_delegated_owned(&registration, &scope.state, parent_ref, input)
        .unwrap();
    record.emit();
    child.wake_owned();
    tokio::time::timeout(Duration::from_secs(5), sink.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    // The real child handoff wins while the captured acceptance projection is
    // deliberately held back at exactly the outside-owner-lock boundary.
    let terminal = permit.association().unwrap();
    assert_eq!(
        terminal.completion.as_ref().unwrap().turns[0].outcome,
        TurnOutcome::Completed
    );
    parent
        .session
        .record_delegation_owned(&origin, &delayed_acceptance);
    assert_eq!(
        parent
            .session
            .tool_get(&caller, &origin.turn_id, &origin.tool_call_id)
            .unwrap()
            .1
            .state
            .delegations
            .as_slice(),
        std::slice::from_ref(&terminal)
    );
    let after = parent
        .session
        .tool_get(&caller, &origin.turn_id, &origin.tool_call_id)
        .unwrap()
        .1;
    assert!(after.state.runner_revision > before.state.runner_revision);
    assert_eq!(
        serde_json::to_value(&after.result).unwrap(),
        serde_json::to_value(&before.result).unwrap()
    );
    *sink.released.lock().unwrap() = true;
    sink.changed.notify_all();
    let notice = MessageInput {
        message_id: MessageId::new().unwrap(),
        text: "actual final notice".into(),
        mode: MessageMode::FollowUp,
        in_reply_to: None,
    };
    let notice_id = notice.message_id.clone();
    let accepted = permit.notify(notice).await.unwrap();
    permit
        .release(crate::DelegationDisposition::Notified(accepted))
        .await
        .unwrap();
    assert_eq!(permit.association(), Some(terminal.clone()));
    let message = parent
        .session
        .state
        .lock()
        .unwrap()
        .messages
        .iter()
        .find(|message| message.input.message_id == notice_id)
        .unwrap()
        .clone();
    assert_eq!(
        message.delegations.as_slice(),
        std::slice::from_ref(&terminal)
    );
    parent.session.resume(&caller).unwrap();
    until(&parent, |state| state.completed.len() == 2).await;
    let saved = records(&parent);
    let notice = saved.iter().find(|row| matches!(&row.entry, HistoryEntry::Input { message_id, .. } if message_id == &notice_id)).unwrap();
    assert_eq!(notice.delegations, [terminal]);
    assert!(saved.iter().any(|row| matches!(&row.entry, HistoryEntry::Runtime { item: HistoryItem::ToolResult(result), .. } if result.call_id.as_str() == call)));
}
impl DelegationSink for HandoffGate {
    fn handoff(&self, completion: DelegationCompletion) -> Result<(), Fault> {
        *self.completion.lock().unwrap() = Some(completion);
        self.entered.add_permits(1);
        let released = self.released.lock().unwrap();
        let (_released, timeout) = self
            .changed
            .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
            .unwrap();
        assert!(!timeout.timed_out(), "test must release result handoff");
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn task071_result_handoff_and_scope_cleanup_are_published_before_waits() {
    let fixture = setup(
        Arc::new(Resources {
            model: model(vec![final_events("child actual result")]),
            tools: ToolSet::default(),
        }),
        |inner| inner,
    );
    let caller = fixture.connection.caller();
    let root = SessionController::new(
        SessionId::new().unwrap(),
        fixture.session.agent_id.clone(),
        fixture.session.host_id.clone(),
        fixture.session.incarnation.clone(),
        SessionConfig::default(),
        fixture.access.clone(),
        Arc::new(Resources {
            model: model(vec![]),
            tools: ToolSet::default(),
        }),
        Arc::new(MemoryHistory),
        vec![],
        Arc::new(AtomicBool::new(true)),
    );
    root.publish_initial();
    let hooks = Arc::new(ScopeGate {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let scope = root.register_scope_owned(hooks.clone()).unwrap();
    let root_ref = SessionRef {
        agent_id: root.agent_id.clone(),
        session_id: root.id.clone(),
    };
    let message = MessageInput {
        message_id: MessageId::new().unwrap(),
        text: "delegated input".into(),
        mode: MessageMode::FollowUp,
        in_reply_to: None,
    };
    let registration = DelegationRegistration {
        host_id: root.host_id.clone(),
        host_incarnation: root.incarnation.clone(),
        root: root_ref.clone(),
        parent: root_ref.clone(),
        child: SessionRef {
            agent_id: fixture.session.agent_id.clone(),
            session_id: fixture.session.id.clone(),
        },
        task_message_id: message.message_id.clone(),
        origin: ToolCallRef {
            turn_id: TurnId::new().unwrap(),
            tool_call_id: ToolCallId("test-registration".into()),
        },
        temporary: false,
        name: None,
    };
    let sink = Arc::new(HandoffGate {
        entered: Semaphore::new(0),
        released: Mutex::new(false),
        changed: Condvar::new(),
        completion: Mutex::new(None),
    });
    let permit = fixture
        .session
        .register_delegation_owned(
            registration.clone(),
            Arc::downgrade(&root),
            scope.state.clone(),
            sink.clone(),
        )
        .unwrap();
    fixture
        .session
        .receive_delegated_owned(&registration, &scope.state, root_ref, message)
        .unwrap()
        .1
        .emit();
    fixture.session.wake_owned();
    tokio::time::timeout(Duration::from_secs(5), sink.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let snapshot = fixture.session.snapshot(&caller).unwrap();
    assert!(snapshot.data.owned_work.result_handoff);
    assert!(snapshot.data.active_turn_id.is_none());
    assert!(snapshot.data.owned_work.tools.is_empty());
    assert!(
        fixture
            .session
            .summary(&caller)
            .unwrap()
            .unwrap()
            .owned_work
            .result_handoff
    );
    *sink.released.lock().unwrap() = true;
    sink.changed.notify_all();
    until(&fixture, |state| !state.handoff_pending).await;
    permit
        .release(crate::DelegationDisposition::NotificationFailed(
            Fault::new(
                FaultKind::SessionLifecycleClosed,
                "test terminates delivery",
            ),
        ))
        .await
        .unwrap();
    let completion = sink.completion.lock().unwrap().take().unwrap();
    assert_eq!(completion.results.len(), 1);
    assert_eq!(
        completion.results[0].info.outcome,
        Some(TurnOutcome::Completed)
    );
    root.close_owned(OperationId::new().unwrap()).unwrap();
    hooks.entered.acquire().await.unwrap().forget();
    let snapshot = root.snapshot(&caller).unwrap();
    assert_eq!(snapshot.data.lifecycle, SessionLifecycle::Closing);
    assert!(snapshot.data.owned_work.scope_cleanup);
    assert_eq!(snapshot.data.end_reason.as_deref(), Some("explicit close"));
    hooks.release.add_permits(1);
    let mut updates = root.subscribe_state(&caller).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while updates.snapshot().unwrap().data.lifecycle != SessionLifecycle::Closed {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(!updates.snapshot().unwrap().data.owned_work.scope_cleanup);
}
