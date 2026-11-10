use super::*;

async fn read(fixture: &Fixture) -> Vec<HistoryItemData> {
    fixture
        .session
        .history_read(
            &fixture.connection.caller(),
            &HistoryReadRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await
        .unwrap()
        .data
        .items
}

#[tokio::test]
async fn pending_partial_read_does_not_confirm_its_save() {
    let gates = (0..3)
        .map(|_| Arc::new(Semaphore::new(0)))
        .collect::<Vec<_>>();
    let (entered, mut saves) = mpsc::unbounded_channel();
    let fixture = setup(
        Arc::new(Resources {
            model: model(vec![vec![ModelEvent::text_delta(
                "unfinished pending content",
            )]]),
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
    send(&fixture, "input");
    assert_eq!(saves.recv().await, Some(0));
    gates[0].add_permits(1);
    assert_eq!(saves.recv().await, Some(1));
    let items = read(&fixture).await;
    assert!(matches!(
        &items[..],
        [
            HistoryItemData::User { .. },
            HistoryItemData::Partial { .. }
        ]
    ));
    {
        let state = fixture.session.state.lock().unwrap();
        let info = &state.active.as_ref().unwrap().info;
        assert_eq!(info.persistence, PersistenceState::Pending);
        assert!(state.history_work.as_ref().is_some_and(|work| work.running));
        assert!(state.history_work.is_some());
        assert!(matches!(&state.history[..], [HistoryItem::User(_)]));
    }
    assert_eq!(records(&fixture).len(), 1, "partial has not been written");
    gates[1].add_permits(1);
    assert_eq!(saves.recv().await, Some(2));
    assert_eq!(
        serde_json::to_value(read(&fixture).await).unwrap(),
        serde_json::to_value(&items).unwrap()
    );
    gates[2].add_permits(1);
    until(&fixture, |state| state.completed.len() == 1).await;
    assert_single_terminal(&fixture, TurnOutcome::Failed, 1);
    assert_eq!(
        serde_json::to_value(read(&fixture).await).unwrap(),
        serde_json::to_value(items).unwrap()
    );
}

#[tokio::test]
async fn failed_partial_read_and_reconcile_keep_one_content_entry() {
    use crate::backend::TestAppendFault::{FullWrite, NoWrite};
    for failure in [NoWrite, FullWrite] {
        let model = model(vec![vec![ModelEvent::text_delta("failed save partial")]]);
        let (fixture, backend, _) = failure_setup(
            model.clone(),
            ToolSet::default(),
            vec![None, Some(failure)],
            None,
        );
        send(&fixture, "input");
        let original = blocked(&fixture).await;
        assert_eq!(original.persistence, PersistenceState::Failed);
        let bytes = std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap();
        let before = serde_json::to_value(read(&fixture).await).unwrap();
        assert_eq!(before.as_array().unwrap().len(), 2);
        assert_eq!(before[1]["kind"], "partial");
        assert_eq!(
            fixture
                .session
                .state
                .lock()
                .unwrap()
                .active
                .as_ref()
                .unwrap()
                .info
                .persistence,
            PersistenceState::Failed
        );
        assert_eq!(backend.appends.load(Ordering::SeqCst), 2);
        assert_eq!(backend.reconciles.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap(),
            bytes
        );
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        until(&fixture, |state| state.completed.len() == 1).await;
        assert_single_terminal(&fixture, TurnOutcome::Failed, 1);
        assert_eq!(serde_json::to_value(read(&fixture).await).unwrap(), before);
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        assert_eq!(serde_json::to_value(read(&fixture).await).unwrap(), before);
        assert_eq!(backend.appends.load(Ordering::SeqCst), 3);
        assert_eq!(backend.reconciles.load(Ordering::SeqCst), 1);
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    }
}
