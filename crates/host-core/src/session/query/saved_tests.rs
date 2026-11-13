//! Actual Session/Runtime and production journal. Async gates and terminal-only
//! seed records below are explicit deterministic scheduling/storage fixtures.
use super::*;
use minicore_host_storage::testing::read_history_bytes;
use tokio::sync::Semaphore;

fn restored(
    fixture: &Fixture,
    backend: Arc<dyn HistoryBackend>,
) -> (Arc<SessionController>, crate::ConnectionHandle) {
    let original = &fixture.session;
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(original.host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let session = SessionController::new(
        original.id.clone(),
        original.agent_id.clone(),
        original.host_id.clone(),
        incarnation,
        original.info(&fixture.connection.caller()).unwrap().config,
        access,
        original.resources_owned().unwrap(),
        backend,
        Vec::new(),
        Arc::new(AtomicBool::new(true)),
    );
    (session, connection)
}
fn seed(fixture: &Fixture) -> TurnId {
    let id = TurnId::new().unwrap();
    let mut journal = fixture.store.as_ref().unwrap().open_journal().unwrap();
    journal
        .accept(vec![
            HistoryEntry::Summary {
                summary: minicore_host_runtime::history::SummaryHistory {
                    content: Text::new("global summary is not owned by this Turn"),
                },
            },
            HistoryEntry::TurnFinished {
                finished_at: minicore_host_protocol::Timestamp::now(),
                requests: 0,
                tool_rounds: 0,
                model_attempts: Vec::new(),
                turn_id: id.clone(),
                outcome: TurnOutcome::Failed,
                reason: Some("explicit terminal-only storage fixture".into()),
            },
        ])
        .unwrap();
    journal.persist_pending().unwrap();
    id
}
fn load(session: &Arc<SessionController>, caller: &CallerContext) {
    session
        .control(
            caller,
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            SessionControlKind::Load,
        )
        .unwrap();
}
async fn loaded(session: &SessionController) {
    let mut snapshots = session.snapshots.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while session.state.lock().unwrap().preparing {
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(session.state.lock().unwrap().residency, Residency::Loaded);
}
fn spawn_read(
    session: &Arc<SessionController>,
    caller: CallerContext,
    id: &TurnId,
) -> tokio::task::JoinHandle<Result<TurnResultSnapshot, Fault>> {
    let session = session.clone();
    let id = id.clone();
    tokio::spawn(async move { session.turn_result_snapshot(&caller, &id).await })
}
struct LoadPublicationGate {
    inner: Arc<JournalHistory>,
    entered: Semaphore,
    release: Semaphore,
}
impl HistoryBackend for LoadPublicationGate {
    fn mode(&self) -> StorageMode {
        self.inner.mode()
    }
    fn load(&self) -> LoadFuture<'_> {
        Box::pin(async {
            let result = self.inner.load().await;
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            result
        })
    }
    fn read_saved_turn(&self, id: &TurnId) -> LoadFuture<'_> {
        self.inner.read_saved_turn(id)
    }
    fn append<'a>(&'a self, entries: Vec<minicore_host_storage::HistoryRecord>) -> SaveFuture<'a> {
        self.inner.append(entries)
    }
}

#[tokio::test]
async fn saved_read_during_loaded_journal_before_owner_publication_never_waits_for_execution() {
    let fixture = fixture(0, true);
    let id = seed(&fixture);
    let backend = Arc::new(LoadPublicationGate {
        inner: fixture.journal.as_ref().unwrap().clone(),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let (session, connection) = restored(&fixture, backend.clone());
    let caller = connection.caller();
    load(&session, &caller);
    tokio::time::timeout(Duration::from_secs(5), backend.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(session.state.lock().unwrap().preparing);
    let saved = tokio::time::timeout(
        Duration::from_secs(1),
        session.turn_result_snapshot(&caller, &id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        saved.data.saved_terminal.unwrap().outcome,
        TurnOutcome::Failed
    );
    assert!(
        saved.data.history.is_empty(),
        "do not assign the unbound global summary to this Turn"
    );
    assert!(session.state.lock().unwrap().completed.is_empty());
    assert!(session.state.lock().unwrap().output.is_empty());
    backend.release.add_permits(1);
    loaded(&session).await;
    assert!(session.turn_result_snapshot(&caller, &id).await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_saved_reader_returns_before_disk_finishes_and_does_not_cancel_load() {
    let fixture = fixture(0, true);
    let id = seed(&fixture);
    let gate = crate::backend::TestReadGate::new();
    fixture
        .journal
        .as_ref()
        .unwrap()
        .gate_cold_read(gate.clone());
    let reader = fixture.session.access.connect().unwrap();
    let read = spawn_read(&fixture.session, reader.caller(), &id);
    gate.reached().await;
    load(&fixture.session, &fixture.connection.caller());
    // Owner/admission state remains available while the actual disk-only guard
    // is held. The independent authorized loader must outlive this caller.
    assert!(fixture.session.state.lock().unwrap().preparing);
    reader.close();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), read)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    gate.release();
    loaded(&fixture.session).await;
    assert!(
        fixture
            .session
            .turn_result_snapshot(&fixture.connection.caller(), &id)
            .await
            .is_ok()
    );
    assert!(fixture.session.state.lock().unwrap().completed.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_reader_obeys_real_resource_retirement_after_inflight_io() {
    let fixture = fixture(0, true);
    let id = seed(&fixture);
    let gate = crate::backend::TestReadGate::new();
    fixture
        .journal
        .as_ref()
        .unwrap()
        .gate_cold_read(gate.clone());
    let read = spawn_read(&fixture.session, fixture.connection.caller(), &id);
    gate.reached().await;
    fixture.session.begin_host_shutdown_owned(false);
    let owner = fixture.session.clone();
    let release = tokio::spawn(async move { owner.release_host_resources_owned().await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !fixture.session.state.lock().unwrap().releasing {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !release.is_finished(),
        "retirement must join the disk reader"
    );
    gate.release();
    assert_eq!(
        read.await.unwrap().unwrap_err().kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
    release.await.unwrap().unwrap();
    assert_eq!(
        fixture
            .session
            .turn_result_snapshot(&fixture.connection.caller(), &id)
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_reader_cannot_return_content_after_owner_reclamation() {
    let fixture = fixture(0, true);
    let id = seed(&fixture);
    let gate = crate::backend::TestReadGate::new();
    fixture
        .journal
        .as_ref()
        .unwrap()
        .gate_cold_read(gate.clone());
    let read = spawn_read(&fixture.session, fixture.connection.caller(), &id);
    gate.reached().await;
    // The actual body-reclamation fence, as used after source deletion. No
    // result-bearing state is synthesized for this scheduling fixture.
    fixture.session.reclaim_bodies();
    gate.release();
    assert_eq!(
        read.await.unwrap().unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
}

#[tokio::test]
async fn saved_results_keep_public_response_indexes_within_one_public_turn() {
    let mut fixture = fixture(3, true);
    let caller = fixture.connection.caller();
    send(&fixture, MessageMode::FollowUp);
    started(&mut fixture).await;
    let id = fixture.session.turn_list(&caller).unwrap().data[0]
        .turn_id
        .clone();
    fixture.events[0]
        .send(ModelEvent::ToolCallStart {
            tool_call_id: "echo-0".parse().unwrap(),
            tool_name: "echo".parse().unwrap(),
        })
        .unwrap();
    fixture.events[0]
        .send(ModelEvent::tool_call_arguments_delta(
            "echo-0".parse().unwrap(),
            "{}",
        ))
        .unwrap();
    fixture.events[0]
        .send(ModelEvent::ToolCallEnd {
            tool_call_id: "echo-0".parse().unwrap(),
        })
        .unwrap();
    fixture.events[0]
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        })
        .unwrap();
    started(&mut fixture).await;
    send(&fixture, MessageMode::Steer);
    fixture.events[1]
        .send(ModelEvent::ProviderReplay {
            replay: minicore_host_runtime::model::ProviderReplay::new(
                "fixture/v1",
                serde_json::json!({"opaque":"complete empty response"}),
            )
            .unwrap(),
        })
        .unwrap();
    finish(&fixture.events[1]);
    started(&mut fixture).await;
    fixture.events[2]
        .send(ModelEvent::text_delta("unfinished second Runtime Loop"))
        .unwrap();
    drop(fixture.events.remove(2));
    until(&fixture, |state| state.completed.contains_key(&id)).await;
    let live = fixture
        .session
        .turn_result_snapshot(&caller, &id)
        .await
        .unwrap();
    let raw = fixture.session.turn_result(&caller, &id).unwrap();
    let loops: Vec<_> = raw
        .history
        .iter()
        .filter_map(|item| {
            if let HistoryItem::User(user) = item {
                Some(user.loop_id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(loops.len(), 2);
    assert_eq!(loops[0], loops[1]);
    assert_eq!(raw.partials[0].partial.request_index, 2);
    assert_eq!(live.data.partials[0].response_index, 2);
    let store = fixture.store.as_ref().unwrap();
    let before = read_history_bytes(store.canonical_path().join("history.jsonl")).unwrap();
    // Create only a new metadata owner and read backend, not an old execution.
    let backend = Arc::new(JournalHistory::new(store.clone()));
    let (session, connection) = restored(&fixture, backend);
    fixture.session.begin_host_shutdown_owned(false);
    fixture
        .session
        .release_host_resources_owned()
        .await
        .unwrap();
    let caller = connection.caller();
    for with_load in [false, true] {
        if with_load {
            load(&session, &caller);
            loaded(&session).await;
        }
        let saved = session.turn_result_snapshot(&caller, &id).await.unwrap();
        assert_eq!(
            saved.data.saved_terminal.as_ref().unwrap().outcome,
            TurnOutcome::Failed
        );
        assert_eq!(
            serde_json::to_value(&saved.data.history).unwrap(),
            serde_json::to_value(&live.data.history).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&saved.data.partials).unwrap(),
            serde_json::to_value(&live.data.partials).unwrap()
        );
        assert_eq!(
            saved
                .data
                .history
                .iter()
                .filter_map(|item| item.response_index)
                .collect::<Vec<_>>(),
            [0, 0, 1]
        );
        assert_eq!(saved.data.partials[0].response_index, 2);
        assert!(saved.source.load_id.is_none());
        assert!(session.state.lock().unwrap().completed.is_empty());
        assert!(session.state.lock().unwrap().output.is_empty());
        assert!(fixture.started.try_recv().is_err());
        assert_public(&saved);
        assert_eq!(
            read_history_bytes(store.canonical_path().join("history.jsonl")).unwrap(),
            before
        );
    }
}
