use super::*;

#[tokio::test]
async fn terminal_claim_failed_report_precedes_cancel_during_original_partial_save() {
    let (fixture, entered, release) = terminal_claim_fixture(
        vec![ModelEvent::text_delta("actual failed stream partial")],
        false,
    );
    send(&fixture, "fail before the save");
    entered.acquire().await.unwrap().forget();
    let before = {
        let state = fixture.session.state.lock().unwrap();
        let active = state.active.as_ref().unwrap();
        assert!(matches!(
            active.report.as_ref().unwrap().outcome,
            LoopOutcome::Failed(_)
        ));
        active.info.clone()
    };
    let (cancel, _) = fixture
        .session
        .cancel(&fixture.connection.caller(), Some(&before.id), false, None)
        .unwrap();
    release.add_permits(1);
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_eq!(before.state, TurnState::Finalizing);
    assert_eq!(before.outcome, Some(TurnOutcome::Failed));
    assert_eq!(cancel.status, CancelStatus::AlreadyEnding);
    let result = fixture
        .session
        .turn_result(&fixture.connection.caller(), &before.id)
        .unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
    assert_eq!(
        result.info.fault.unwrap().kind(),
        Some(FaultKind::TurnModelStreamInterrupted)
    );
    assert!(result.info.cleanup_faults.is_empty());
    assert_single_terminal(&fixture, TurnOutcome::Failed, 1);
}

#[tokio::test]
async fn terminal_claim_cancel_precedes_later_runtime_history_failure() {
    let (fixture, entered, release) =
        terminal_claim_fixture(final_events("actual complete response"), true);
    send(&fixture, "cancel before the save fails");
    entered.acquire().await.unwrap().forget();
    let turn = {
        let state = fixture.session.state.lock().unwrap();
        let active = state.active.as_ref().unwrap();
        assert!(
            active.report.is_none(),
            "Runtime still owns its history callback"
        );
        active.info.id.clone()
    };
    let (cancel, _) = fixture
        .session
        .cancel(&fixture.connection.caller(), Some(&turn), false, None)
        .unwrap();
    assert_eq!(cancel.status, CancelStatus::Claimed);
    release.add_permits(1);
    let original = blocked(&fixture).await;
    {
        let state = fixture.session.state.lock().unwrap();
        let report = state.active.as_ref().unwrap().report.as_ref().unwrap();
        assert!(
            matches!(&report.outcome, LoopOutcome::Cancelled(_))
                && report.history_failure.is_some()
        );
    }
    assert_eq!(original.outcome, Some(TurnOutcome::Cancelled));
    assert!(original.fault.is_none());
    assert_eq!(
        original.persistence_fault.as_ref().unwrap().kind(),
        Some(FaultKind::SessionHistoryWriteFailed)
    );
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_single_terminal(&fixture, TurnOutcome::Cancelled, 0);
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.get(&turn).unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert!(result.info.fault.is_none());
    assert!(result.info.persistence_fault.is_none());
}

#[tokio::test]
async fn terminal_claim_cancel_precedes_real_runtime_panic_report() {
    let entered = Arc::new(Semaphore::new(0));
    let fixture = setup(
        Arc::new(Resources {
            model: Arc::new(CancelDropPanicModel {
                descriptor: ModelDescriptor::new(
                    "test/model".parse().unwrap(),
                    128_000,
                    BTreeSet::from([ReasoningPreference::Auto]),
                    true,
                )
                .unwrap(),
                entered: entered.clone(),
            }),
            tools: ToolSet::default(),
        }),
        |inner| inner,
    );
    send(&fixture, "cancel the actual Runtime stream");
    entered.acquire().await.unwrap().forget();
    let turn = fixture
        .session
        .state
        .lock()
        .unwrap()
        .active
        .as_ref()
        .unwrap()
        .info
        .id
        .clone();
    let (cancel, _) = fixture
        .session
        .cancel(&fixture.connection.caller(), Some(&turn), false, None)
        .unwrap();
    assert_eq!(cancel.status, CancelStatus::Claimed);
    until(&fixture, |state| state.completed.len() == 1).await;
    let result = fixture
        .session
        .turn_result(&fixture.connection.caller(), &turn)
        .unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert!(result.info.fault.is_none());
    assert_eq!(result.info.cleanup_faults.len(), 1);
    assert_eq!(
        result.info.cleanup_faults[0].kind(),
        Some(FaultKind::TurnInternalUnexpected)
    );
}
