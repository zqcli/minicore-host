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
    // Install the real metadata owner, then finish the paused-input checkpoint
    // before binding/model acceptance so it cannot mask a missing history one.
    let record = fixture.store.load().unwrap();
    *fixture.session.store.lock().unwrap() = Some(fixture.store.clone());
    {
        let mut state = fixture.session.state.lock().unwrap();
        state.info.created_at = record.data.created_at;
        state.info.last_activity_at = record.data.last_activity_at;
        state.record_base = Some(record);
    }
    fixture.session.pause(&fixture.connection.caller()).unwrap();
    send(&fixture, "run tool");
    until(&fixture, |state| state.record_work.is_none()).await;
    let input_checkpoint = fixture.store.load().unwrap().data.last_activity_at;
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    assert_eq!(rx.recv().await, Some(0));
    // The real history backend is gated before append/settlement. Activity for
    // the newly accepted response already owns metadata work or is confirmed.
    {
        let state = fixture.session.state.lock().unwrap();
        let accepted = state
            .readable_history
            .as_ref()
            .unwrap()
            .last()
            .unwrap()
            .recorded_at;
        assert!(accepted > input_checkpoint);
        assert!(
            state.record_work.is_some()
                || state.record_base.as_ref().unwrap().data.last_activity_at >= accepted
        );
    }
    until(&fixture, |state| state.record_work.is_none()).await;
    assert_eq!(
        fixture.store.load().unwrap().data.last_activity_at,
        fixture
            .session
            .info(&fixture.connection.caller())
            .unwrap()
            .last_activity_at
    );
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
async fn protocol_complete_metadata_does_not_execute_or_reload_failed_tool_calls() {
    use minicore_host_runtime::model::ProviderReplay;

    let first_id = "valid-call".parse().unwrap();
    let second_id = "invalid-call".parse().unwrap();
    let invalid_arguments = "{\"value\":\"bad\u{1b}text\"}";
    let replay = ProviderReplay::new(
        "minicore-host.openai-chat.v1",
        serde_json::json!({"finish_reason": "tool_calls"}),
    )
    .unwrap();
    let events = vec![
        ModelEvent::ToolCallStart {
            tool_call_id: first_id,
            tool_name: "count".parse().unwrap(),
        },
        ModelEvent::tool_call_arguments_delta("valid-call".parse().unwrap(), "{}"),
        ModelEvent::ToolCallStart {
            tool_call_id: second_id,
            tool_name: "count".parse().unwrap(),
        },
        ModelEvent::tool_call_arguments_delta("invalid-call".parse().unwrap(), invalid_arguments),
        ModelEvent::ProviderReplay {
            replay: replay.clone(),
        },
        ModelEvent::ToolCallEnd {
            tool_call_id: "valid-call".parse().unwrap(),
        },
        ModelEvent::ToolCallEnd {
            tool_call_id: "invalid-call".parse().unwrap(),
        },
        ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        },
    ];
    let model = model(vec![events, final_events("unexpected continuation")]);
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
    let fixture = setup(
        Arc::new(Resources {
            model: model.clone(),
            tools: builder.build().unwrap(),
        }),
        |inner| inner,
    );
    send(&fixture, "preserve the failed response");
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_eq!(tool_calls.load(Ordering::SeqCst), 0);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    let expected = {
        let state = fixture.session.state.lock().unwrap();
        let result = state.completed.values().next().unwrap();
        assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
        assert_eq!(result.info.persistence, PersistenceState::Persisted);
        assert_eq!(result.partials.len(), 1);
        let partial = &result.partials[0].partial;
        assert_eq!(partial.provider_replay.as_ref(), Some(&replay));
        assert_eq!(partial.content.len(), 1);
        assert_eq!(
            partial.content[0]
                .as_tool_call()
                .unwrap()
                .tool_call_id()
                .as_str(),
            "valid-call"
        );
        assert_eq!(partial.incomplete_tool_calls.len(), 1);
        assert_eq!(
            partial.incomplete_tool_calls[0].arguments_text,
            invalid_arguments
        );
        assert!(matches!(&state.history[..], [HistoryItem::User(_)]));
        partial.clone()
    };
    fixture.session.begin_host_shutdown_owned(false);
    fixture
        .session
        .release_host_resources_owned()
        .await
        .unwrap();
    let loaded = fixture.store.read_history().unwrap();
    assert_eq!(loaded.records.len(), 3);
    assert!(matches!(
        &loaded.records[1].entry,
        HistoryEntry::Partial { partial, .. } if partial == &expected
    ));
    assert!(matches!(
        &loaded.runtime_history()[..],
        [HistoryItem::User(_)]
    ));
    assert_eq!(tool_calls.load(Ordering::SeqCst), 0);
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
                1 + usize::from(boundary >= 1) + usize::from(boundary == 2)
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
    assert!(active.pending_partial.is_some());
    assert!(active.info.persistence_fault.is_some());
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

#[tokio::test]
async fn usage_only_failed_request_is_saved_and_cold_read_without_invented_text() {
    let usage = minicore_host_runtime::model::Usage::from_optional(Some(19), None, None);
    let fixture = setup(
        Arc::new(Resources {
            model: model(vec![vec![ModelEvent::Usage { usage }]]),
            tools: ToolSet::default(),
        }),
        |inner| inner,
    );
    send(&fixture, "record actual usage");
    until(&fixture, |state| state.completed.len() == 1).await;
    let turn_id = {
        let state = fixture.session.state.lock().unwrap();
        let result = state.completed.values().next().unwrap();
        assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
        assert_eq!(result.partials.len(), 1);
        let partial = &result.partials[0].partial;
        assert!(partial.content.is_empty());
        assert!(partial.incomplete_tool_calls.is_empty());
        assert_eq!(partial.usage, Some(usage));
        result.info.id.clone()
    };
    let stored = records(&fixture);
    assert!(
        matches!(&stored[1].entry, HistoryEntry::Partial { partial, .. } if partial.usage == Some(usage) && partial.content.is_empty())
    );
    let original = &fixture.session;
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(original.host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let cold = SessionController::new(
        original.id.clone(),
        original.agent_id.clone(),
        original.host_id.clone(),
        incarnation,
        original.info(&fixture.connection.caller()).unwrap().config,
        access,
        original.resources_owned().unwrap(),
        Arc::new(JournalHistory::new(fixture.store.clone())),
        Vec::new(),
        Arc::new(AtomicBool::new(true)),
    );
    original.begin_host_shutdown_owned(false);
    original.release_host_resources_owned().await.unwrap();
    let saved = cold
        .turn_result_snapshot(&connection.caller(), &turn_id)
        .await
        .unwrap();
    assert_eq!(saved.data.partials.len(), 1);
    assert!(saved.data.partials[0].partial.content.is_empty());
    assert_eq!(
        saved.data.partials[0]
            .partial
            .usage
            .as_ref()
            .unwrap()
            .input_tokens(),
        Some(19)
    );
    assert!(cold.state.lock().unwrap().active.is_none());
}
