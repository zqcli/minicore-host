use super::*;
use minicore_host_storage::testing::{read_history_bytes, read_history_text};

#[tokio::test]
async fn resume_confirms_original_zero_or_full_write_without_second_model_or_tool() {
    use crate::backend::TestAppendFault::{FullWrite, NoWrite};
    for failure in [NoWrite, FullWrite] {
        let call_id = "must-not-execute".parse().unwrap();
        let model = model(vec![vec![
            ModelEvent::ToolCallStart {
                tool_call_id: call_id,
                tool_name: "count".parse().unwrap(),
            },
            ModelEvent::tool_call_arguments_delta("must-not-execute".parse().unwrap(), "{}"),
            ModelEvent::ToolCallEnd {
                tool_call_id: "must-not-execute".parse().unwrap(),
            },
            ModelEvent::Finish {
                reason: ModelFinishReason::ToolCalls,
            },
        ]]);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tools = ToolSet::builder();
        tools.register(CountTool {
            spec: ToolSpec::new(
                "count".parse().unwrap(),
                "count fixture",
                serde_json::json!({}),
            )
            .unwrap(),
            calls: calls.clone(),
        });
        let (fixture, backend, _) = failure_setup(
            model.clone(),
            tools.build().unwrap(),
            vec![Some(failure)],
            None,
        );
        send(&fixture, "original range");
        let original = blocked(&fixture).await;
        assert_eq!(original.outcome, Some(TurnOutcome::Failed));
        assert!(
            original.fault.is_none(),
            "history failure is not a fabricated model fault"
        );
        let before =
            read_history_bytes(fixture.store.canonical_path().join("history.jsonl")).unwrap();
        assert_eq!(backend.appends.load(Ordering::SeqCst), 1);
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        until(&fixture, |state| state.completed.len() == 1).await;
        assert_single_terminal(&fixture, TurnOutcome::Failed, 0);
        let after =
            read_history_bytes(fixture.store.canonical_path().join("history.jsonl")).unwrap();
        assert!(after.starts_with(&before));
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(backend.appends.load(Ordering::SeqCst), 2);
        assert_eq!(backend.reconciles.load(Ordering::SeqCst), 1);
        // Repeated resume with no pending original save never appends another end.
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        assert_eq!(
            read_history_bytes(fixture.store.canonical_path().join("history.jsonl")).unwrap(),
            after
        );
    }
}
#[tokio::test]
async fn report_partial_and_terminal_each_continue_once_and_keep_primary_failure() {
    use crate::backend::TestAppendFault::{FullWrite, NoWrite};
    let model = model(vec![vec![ModelEvent::text_delta(
        "actual unfinished output",
    )]]);
    let (fixture, backend, _) = failure_setup(
        model.clone(),
        ToolSet::default(),
        vec![Some(NoWrite), Some(FullWrite), Some(FullWrite)],
        None,
    );
    send(&fixture, "fail during stream and each save");
    let original = blocked(&fixture).await;
    assert_eq!(
        original.fault.as_ref().unwrap().kind(),
        Some(FaultKind::TurnModelStreamInterrupted)
    );
    for expected_appends in [2, 3] {
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        until(&fixture, |state| {
            !state.history_work.as_ref().is_some_and(|work| work.running)
                && state.history_work.is_some()
                && backend.appends.load(Ordering::SeqCst) == expected_appends
        })
        .await;
        let active = fixture
            .session
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .unwrap()
            .info
            .clone();
        assert_eq!(active.id, original.id);
        assert_eq!(active.load_id, original.load_id);
        assert_eq!(active.outcome, original.outcome);
        assert_eq!(
            active.fault.as_ref().unwrap().kind(),
            original.fault.as_ref().unwrap().kind()
        );
        assert_eq!(active.persistence, PersistenceState::Failed);
    }
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_single_terminal(&fixture, TurnOutcome::Failed, 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.appends.load(Ordering::SeqCst), 3);
    assert_eq!(backend.reconciles.load(Ordering::SeqCst), 3);
    assert_eq!(records(&fixture).len(), 3);
}
#[tokio::test]
async fn terminal_save_continuation_preserves_completed_claim_despite_force() {
    use crate::backend::TestAppendFault::FullWrite;
    let model = model(vec![final_events("complete response")]);
    let (fixture, backend, _) = failure_setup(
        model.clone(),
        ToolSet::default(),
        vec![None, Some(FullWrite)],
        None,
    );
    send(&fixture, "complete then terminal fault");
    let original = blocked(&fixture).await;
    assert_eq!(original.outcome, Some(TurnOutcome::Completed));
    fixture.session.begin_host_shutdown_owned(true);
    assert!(!fixture.session.shutdown_status_owned().quiescent);
    assert!(
        fixture
            .session
            .release_host_resources_owned()
            .await
            .is_err()
    );
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_single_terminal(&fixture, TurnOutcome::Completed, 0);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.appends.load(Ordering::SeqCst), 2);
    fixture
        .session
        .release_host_resources_owned()
        .await
        .unwrap();
    assert_eq!(
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
}
#[tokio::test]
async fn abandoned_waiter_revoked_connection_and_force_do_not_drop_original_continuation() {
    use crate::backend::TestAppendFault::FullWrite;
    let model = model(vec![final_events("already complete")]);
    let gate = Arc::new(Semaphore::new(0));
    let (fixture, backend, mut entered) = failure_setup(
        model.clone(),
        ToolSet::default(),
        vec![Some(FullWrite)],
        Some(gate.clone()),
    );
    send(&fixture, "save survives callers");
    let original = blocked(&fixture).await;
    let waiter_owner = fixture.session.clone();
    let waiter_caller = fixture.connection.caller();
    let id = original.id.clone();
    let waiter = tokio::spawn(async move { waiter_owner.wait_turn(&waiter_caller, &id).await });
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    entered.recv().await.unwrap();
    assert_eq!(
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionBusy)
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    fixture.connection.close();
    fixture.session.begin_host_shutdown_owned(true);
    assert!(!fixture.session.shutdown_status_owned().drained);
    gate.add_permits(1);
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_single_terminal(&fixture, original.outcome.unwrap(), 0);
    assert_eq!(original.outcome, Some(TurnOutcome::Completed));
    assert_eq!(backend.reconciles.load(Ordering::SeqCst), 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    fixture
        .session
        .release_host_resources_owned()
        .await
        .unwrap();
}
#[tokio::test]
async fn partial_or_changed_original_bytes_remain_blocked_and_unchanged() {
    use crate::backend::TestAppendFault::{FullWrite, Prefix};
    for change in [false, true] {
        let model = model(vec![final_events("original content")]);
        let (fixture, backend, _) = failure_setup(
            model.clone(),
            ToolSet::default(),
            vec![Some(if change { FullWrite } else { Prefix(17) })],
            None,
        );
        send(&fixture, "original input");
        let original = blocked(&fixture).await;
        let path = fixture.store.canonical_path().join("history.jsonl");
        if change {
            let bytes = read_history_text(&path).unwrap();
            backend
                .inner
                .overwrite_owned_history_for_test(
                    bytes.replace("original input", "modified input").as_bytes(),
                )
                .unwrap();
        }
        let before = read_history_bytes(&path).unwrap();
        for attempt in 1..=2 {
            fixture
                .session
                .resume(&fixture.connection.caller())
                .unwrap();
            until(&fixture, |state| {
                !state.history_work.as_ref().is_some_and(|work| work.running)
                    && backend.reconciles.load(Ordering::SeqCst) == attempt
            })
            .await;
            assert_eq!(read_history_bytes(&path).unwrap(), before);
            let state = fixture.session.state.lock().unwrap();
            let active = state.active.as_ref().unwrap();
            assert_eq!(active.info.id, original.id);
            assert_eq!(active.info.load_id, original.load_id);
            assert_eq!(active.info.persistence, PersistenceState::Failed);
            assert!(state.completed.is_empty());
        }
        assert_eq!(backend.appends.load(Ordering::SeqCst), 1);
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        fixture.session.begin_host_shutdown_owned(true);
        assert!(!fixture.session.shutdown_status_owned().quiescent);
        assert!(
            fixture
                .session
                .release_host_resources_owned()
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn force_keeps_cancelled_partial_and_original_save_through_continuation() {
    use crate::backend::TestAppendFault::FullWrite;
    let consumed_delta = Arc::new(Semaphore::new(0));
    let model = Arc::new(PartialUntilCancelled {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128_000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        consumed_delta: consumed_delta.clone(),
    });
    let (fixture, backend, _) =
        failure_setup(model, ToolSet::default(), vec![Some(FullWrite)], None);
    send(&fixture, "force during actual streaming");
    consumed_delta.acquire().await.unwrap().forget();
    fixture.session.begin_host_shutdown_owned(true);
    let original = blocked(&fixture).await;
    assert_eq!(original.outcome, Some(TurnOutcome::Cancelled));
    assert!(original.fault.is_none());
    assert!(!fixture.session.shutdown_status_owned().quiescent);
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_single_terminal(&fixture, TurnOutcome::Cancelled, 1);
    assert_eq!(backend.appends.load(Ordering::SeqCst), 3);
    let saved = records(&fixture);
    assert!(matches!(&saved.last().unwrap().entry,
        HistoryEntry::TurnFinished { reason: Some(reason), .. } if reason.contains("Shutdown")));
    fixture
        .session
        .release_host_resources_owned()
        .await
        .unwrap();
}

#[tokio::test]
async fn confirmed_tool_execution_is_not_repeated_after_its_save_fails() {
    use crate::backend::TestAppendFault::FullWrite;
    let first = vec![
        ModelEvent::ToolCallStart {
            tool_call_id: "executed-once".parse().unwrap(),
            tool_name: "count".parse().unwrap(),
        },
        ModelEvent::tool_call_arguments_delta("executed-once".parse().unwrap(), "{}"),
        ModelEvent::ToolCallEnd {
            tool_call_id: "executed-once".parse().unwrap(),
        },
        ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        },
    ];
    let model = model(vec![first, final_events("must not be requested")]);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolSet::builder();
    tools.register(CountTool {
        spec: ToolSpec::new(
            "count".parse().unwrap(),
            "count fixture",
            serde_json::json!({}),
        )
        .unwrap(),
        calls: calls.clone(),
    });
    let (fixture, backend, _) = failure_setup(
        model.clone(),
        tools.build().unwrap(),
        vec![None, Some(FullWrite)],
        None,
    );
    send(&fixture, "execute once");
    let original = blocked(&fixture).await;
    assert_eq!(original.outcome, Some(TurnOutcome::Failed));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_single_terminal(&fixture, TurnOutcome::Failed, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.appends.load(Ordering::SeqCst), 3);
    assert_eq!(
        records(&fixture)
            .iter()
            .filter(|record| matches!(
                record.entry,
                HistoryEntry::Runtime {
                    item: HistoryItem::ToolResult(_),
                    ..
                }
            ))
            .count(),
        1
    );
}
