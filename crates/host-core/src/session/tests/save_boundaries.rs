use super::*;

#[tokio::test]
async fn real_files_confirm_response_tool_and_turn_boundaries_before_continuation() {
    let call_id = "tool-original-id".parse().unwrap();
    let first = vec![
        ModelEvent::ToolCallStart {
            tool_call_id: call_id,
            tool_name: "count".parse().unwrap(),
        },
        ModelEvent::tool_call_arguments_delta("tool-original-id".parse().unwrap(), "{}"),
        ModelEvent::ToolCallEnd {
            tool_call_id: "tool-original-id".parse().unwrap(),
        },
        ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        },
    ];
    let model = model(vec![first, final_events("done")]);
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let mut builder = ToolSet::builder();
    builder.register(CountTool {
        spec: ToolSpec::new(
            "count".parse().unwrap(),
            "count fixture calls",
            serde_json::json!({}),
        )
        .unwrap(),
        calls: tool_calls.clone(),
    });
    let resources = Arc::new(Resources {
        model: model.clone(),
        tools: builder.build().unwrap(),
    });
    let gates = (0..4)
        .map(|_| Arc::new(Semaphore::new(0)))
        .collect::<Vec<_>>();
    let (entered, mut rx) = mpsc::unbounded_channel();
    let fixture = setup(resources, |inner| {
        Arc::new(GatedJournal {
            inner,
            gates: Mutex::new(gates.iter().cloned().collect()),
            entered,
            count: AtomicUsize::new(0),
        })
    });
    send(&fixture, "run tool");
    assert_eq!(rx.recv().await, Some(0));
    assert_eq!(tool_calls.load(Ordering::SeqCst), 0);
    assert!(records(&fixture).is_empty());
    // Receipt/query/queue mutation remains available while actual save work is held.
    fixture.session.pause(&fixture.connection.caller()).unwrap();
    assert!(
        fixture
            .session
            .snapshot(&fixture.connection.caller())
            .unwrap()
            .data
            .saving
    );
    gates[0].add_permits(1);
    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(tool_calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(records(&fixture).len(), 2);
    gates[1].add_permits(1);
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
    assert_eq!(records(&fixture).len(), 3);
    gates[2].add_permits(1);
    assert_eq!(rx.recv().await, Some(3));
    assert_eq!(records(&fixture).len(), 4);
    {
        let state = fixture.session.state.lock().unwrap();
        assert!(state.completed.is_empty());
        assert_eq!(
            state.active.as_ref().unwrap().info.state,
            TurnState::Finalizing
        );
    }
    gates[3].add_permits(1);
    until(&fixture, |state| state.completed.len() == 1).await;
    let saved = records(&fixture);
    assert_eq!(saved.len(), 5);
    assert_eq!(
        saved.iter().map(|r| r.position).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.values().next().unwrap();
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert_eq!(result.info.outcome, Some(TurnOutcome::Completed));
    assert_eq!(state.history.len(), 4);
}

#[tokio::test]
async fn real_partial_is_saved_as_partial_without_fabricated_assistant() {
    let model = model(vec![vec![ModelEvent::text_delta("unfinished actual text")]]);
    let fixture = setup(
        Arc::new(Resources {
            model,
            tools: ToolSet::default(),
        }),
        |inner| inner,
    );
    send(&fixture, "respond");
    until(&fixture, |state| state.completed.len() == 1).await;
    let saved = records(&fixture);
    assert_eq!(saved.len(), 3);
    assert!(matches!(&saved[0].entry, HistoryEntry::Input { .. }));
    assert!(matches!(&saved[1].entry, HistoryEntry::Partial { .. }));
    assert!(matches!(
        &saved[2].entry,
        HistoryEntry::TurnFinished {
            outcome: TurnOutcome::Failed,
            ..
        }
    ));
    assert!(!saved.iter().any(|record| matches!(
        record.entry,
        HistoryEntry::Runtime {
            item: HistoryItem::Assistant(_),
            ..
        }
    )));
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.values().next().unwrap();
    assert_eq!(result.partials.len(), 1);
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert!(state.failure_paused);
}

#[tokio::test]
async fn partial_and_terminal_acceptance_register_original_saves_before_gated_io() {
    let model = model(vec![vec![ModelEvent::text_delta("unfinished actual text")]]);
    let gates = (0..3)
        .map(|_| Arc::new(Semaphore::new(0)))
        .collect::<Vec<_>>();
    let (entered, mut rx) = mpsc::unbounded_channel();
    let fixture = setup(
        Arc::new(Resources {
            model: model.clone(),
            tools: ToolSet::default(),
        }),
        |inner| {
            Arc::new(GatedJournal {
                inner,
                gates: Mutex::new(gates.iter().cloned().collect()),
                entered,
                count: AtomicUsize::new(0),
            })
        },
    );
    send(&fixture, "respond partially");
    for (boundary, gate) in gates.iter().enumerate() {
        assert_eq!(rx.recv().await, Some(boundary));
        let context = fixture
            .session
            .context(&fixture.connection.caller())
            .await
            .unwrap();
        assert!(context.data.history_save.pending && context.data.history_save.running);
        assert!(context.data.history_save.fault.is_none());
        let history = context.data.history.unwrap();
        assert_eq!(history.position, boundary as u64 + 1);
        assert_eq!(history.raw_items, 1);
        assert!(history.latest_response.is_none());
        assert_eq!(records(&fixture).len(), boundary);
        {
            let state = fixture.session.state.lock().unwrap();
            let active = state.active.as_ref().unwrap();
            assert_eq!(active.info.persistence, PersistenceState::Pending);
            assert_eq!(active.partial_accepted, boundary >= 1);
            assert_eq!(active.partials.len(), usize::from(boundary >= 1));
            assert_eq!(active.terminal_accepted, boundary == 2);
            assert_eq!(
                state.readable_history.as_ref().unwrap().len(),
                1 + usize::from(boundary >= 1)
            );
            assert!(state.completed.is_empty());
        }
        gate.add_permits(1);
    }
    until(&fixture, |state| state.completed.len() == 1).await;
    let saved = records(&fixture);
    assert_eq!(saved.len(), 3);
    assert!(matches!(saved[0].entry, HistoryEntry::Input { .. }));
    assert!(matches!(saved[1].entry, HistoryEntry::Partial { .. }));
    assert!(matches!(
        saved[2].entry,
        HistoryEntry::TurnFinished {
            outcome: TurnOutcome::Failed,
            ..
        }
    ));
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.values().next().unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert_eq!(result.partials.len(), 1);
    assert!(state.history_work.is_none());
}

#[tokio::test]
async fn failed_save_retains_report_and_primary_failure_without_reappend_or_success() {
    let model = model(vec![vec![ModelEvent::text_delta(
        "partial before primary stream failure",
    )]]);
    let captured = Arc::new(Mutex::new(None));
    let out = captured.clone();
    let fixture = setup(
        Arc::new(Resources {
            model: model.clone(),
            tools: ToolSet::default(),
        }),
        |inner| {
            let fail = Arc::new(FailedSave {
                count: AtomicUsize::new(0),
                inner,
            });
            *out.lock().unwrap() = Some(fail.clone());
            fail
        },
    );
    send(&fixture, "fail twice");
    until(&fixture, |state| {
        !state.driver_running && state.block_reason.is_some()
    })
    .await;
    assert_eq!(
        captured
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .count
            .load(Ordering::SeqCst),
        1
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert!(records(&fixture).is_empty());
    let state = fixture.session.state.lock().unwrap();
    let active = state.active.as_ref().unwrap();
    assert_eq!(active.info.state, TurnState::Finalizing);
    assert_eq!(active.info.outcome, Some(TurnOutcome::Failed));
    assert_eq!(active.info.persistence, PersistenceState::Failed);
    assert_eq!(
        active.info.fault.as_ref().unwrap().kind(),
        Some(FaultKind::TurnModelStreamInterrupted)
    );
    assert_eq!(
        active.info.persistence_fault.as_ref().unwrap().kind(),
        Some(FaultKind::SessionHistoryWriteFailed)
    );
    assert!(active.report.as_ref().unwrap().partial.is_some());
    assert!(active.report.as_ref().unwrap().history_failure.is_some());
    assert!(state.completed.is_empty());
    assert_eq!(state.history.len(), 1);
}

#[tokio::test]
async fn cancellation_during_save_keeps_save_owner_and_complete_response() {
    let model = model(vec![final_events("already complete")]);
    let gate = Arc::new(Semaphore::new(0));
    let (entered, mut rx) = mpsc::unbounded_channel();
    let fixture = setup(
        Arc::new(Resources {
            model,
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
    send(&fixture, "complete then cancel");
    assert_eq!(rx.recv().await, Some(0));
    assert_eq!(
        fixture
            .session
            .cancel(&fixture.connection.caller(), None, false, None)
            .unwrap()
            .0
            .status,
        CancelStatus::Claimed
    );
    assert!(fixture.session.state.lock().unwrap().completed.is_empty());
    assert!(DataRoot::acquire(fixture.store.canonical_path().ancestors().nth(4).unwrap()).is_err());
    gate.add_permits(1);
    until(&fixture, |state| state.completed.len() == 1).await;
    let saved = records(&fixture);
    assert_eq!(saved.len(), 3);
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.values().next().unwrap();
    assert!(result.partials.is_empty());
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert!(
        fixture
            .access
            .snapshot(&fixture.connection.caller())
            .unwrap()
            .occupied
    );
}

#[tokio::test]
async fn synchronous_backend_panics_are_failed_saves_not_lost_owners() {
    let fixture = setup(
        Arc::new(Resources {
            model: model(vec![final_events("complete fixture")]),
            tools: ToolSet::default(),
        }),
        |_| Arc::new(ConstructorPanicHistory),
    );
    send(&fixture, "capture constructor panic");
    let original = blocked(&fixture).await;
    assert_eq!(original.persistence, PersistenceState::Failed);
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    until(&fixture, |state| {
        !state.history_work.as_ref().is_some_and(|work| work.running)
    })
    .await;
    let state = fixture.session.state.lock().unwrap();
    assert!(state.history_work.is_some());
    assert!(state.completed.is_empty());
    assert_eq!(
        state.active.as_ref().unwrap().info.outcome,
        original.outcome
    );
}
