//! Deterministic scheduling fixtures around the production journal and Session.
//! No live provider or TUI validation is claimed.
use super::*;
use crate::JournalHistory;
use minicore_host_storage::{AgentData, AgentRecord, DataRoot, HostRecord, SessionData};
use minicore_runtime::{
    history::SummaryHistory,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use std::{collections::BTreeSet, sync::atomic::AtomicUsize, time::Duration};
use tokio::sync::Semaphore;

struct ModelFixture {
    descriptor: ModelDescriptor,
    calls: AtomicUsize,
}
impl Model for ModelFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(Box::pin(futures_util::stream::iter([
                Ok(ModelEvent::text_delta("new owner response")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
struct Resources(Arc<ModelFixture>);
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.0.clone(),
                    ReasoningPreference::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
fn resources() -> (Arc<dyn ResourceResolver>, Arc<ModelFixture>) {
    let model = Arc::new(ModelFixture {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128_000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        calls: AtomicUsize::new(0),
    });
    (Arc::new(Resources(model.clone())), model)
}
struct Gate {
    entered: Semaphore,
    released: Semaphore,
}
impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            released: Semaphore::new(0),
        })
    }
    async fn wait(&self) {
        self.entered.add_permits(1);
        self.released.acquire().await.unwrap().forget();
    }
    async fn reached(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
    fn release(&self) {
        self.released.add_permits(1);
    }
}
struct GatedLoadFixture {
    inner: Arc<JournalHistory>,
    gate: Arc<Gate>,
}
impl HistoryBackend for GatedLoadFixture {
    fn mode(&self) -> StorageMode {
        self.inner.mode()
    }
    fn load(&self) -> LoadFuture<'_> {
        Box::pin(async move {
            let result = self.inner.load().await;
            self.gate.wait().await;
            result
        })
    }
    fn read_cold(&self) -> HistoryReadFuture<'_> {
        self.inner.read_cold()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        self.inner.append(entries)
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        self.inner.reconcile()
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    store: SessionStore,
    session: Arc<SessionController>,
    connection: crate::ConnectionHandle,
    model: Arc<ModelFixture>,
}
fn summary(text: &str) -> HistoryEntry {
    HistoryEntry::Summary {
        summary: SummaryHistory {
            content: Text::new(text),
        },
    }
}
fn setup(wrap: impl FnOnce(Arc<JournalHistory>) -> Arc<dyn HistoryBackend>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(dir.path()).unwrap();
    let host_id = HostId::new().unwrap();
    let host = root.host(host_id.clone());
    host.save_host(&HostRecord::new(
        host_id.clone(),
        crate::HostConfig::default(),
        0,
    ))
    .unwrap();
    let agent_id = AgentId::new().unwrap();
    let agent = host.agent(agent_id.clone(), StorageMode::Persistent);
    agent
        .save(&AgentRecord::new(
            agent_id.clone(),
            host_id.clone(),
            AgentData {
                name: "task052 fixture".into(),
                mode: StorageMode::Persistent,
                config: AgentConfig::default(),
                config_revision: 0,
                default_session_id: None,
            },
        ))
        .unwrap();
    let id = SessionId::new().unwrap();
    let store = agent.session(id.clone(), StorageMode::Persistent).unwrap();
    let config = SessionConfig {
        model: "test/model".parse().unwrap(),
        ..SessionConfig::default()
    };
    store
        .save(&SessionRecord::new(
            id.clone(),
            agent_id.clone(),
            SessionData {
                title: None,
                mode: StorageMode::Persistent,
                agent_config_revision: 0,
                config_revision: 0,
                effective_config: config.clone(),
                parent: None,
                closed: None,
            },
        ))
        .unwrap();
    let mut journal = store.open_journal().unwrap();
    journal.accept(vec![summary("persisted seed")]).unwrap();
    journal.persist_pending().unwrap();
    drop(journal);
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let (resources, model) = resources();
    let session = SessionController::new(
        id,
        agent_id,
        host_id,
        incarnation,
        config,
        access,
        resources,
        wrap(Arc::new(JournalHistory::new(store.clone()))),
        Vec::new(),
        Arc::new(AtomicBool::new(true)),
    );
    *session.store.lock().unwrap() = Some(store.clone());
    Fixture {
        _dir: dir,
        store,
        session,
        connection,
        model,
    }
}
fn read_request() -> HistoryReadRequest {
    HistoryReadRequest {
        cursor: None,
        limit: 100,
    }
}
fn load(fixture: &Fixture) {
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
}
async fn settled(fixture: &Fixture) {
    let mut snapshots = fixture.session.snapshots.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.session.state.lock().unwrap().preparing {
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn load_winner_history_read_uses_owner_publication() {
    let gate = Gate::new();
    let fixture = setup(|inner| {
        Arc::new(GatedLoadFixture {
            inner,
            gate: gate.clone(),
        })
    });
    load(&fixture);
    gate.reached().await;
    let caller = fixture.connection.caller();
    let request = read_request();
    let read = fixture.session.history_read(&caller, &request);
    tokio::pin!(read);
    assert!(futures_util::poll!(read.as_mut()).is_pending());
    gate.release();
    let page = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&page.data.items[..], [HistoryItemData::Summary { text }] if text == "persisted seed")
    );
    settled(&fixture).await;
    assert!(fixture.session.state.lock().unwrap().block_reason.is_none());
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
}

struct GatedReadFixture {
    inner: Arc<dyn HistoryBackend>,
    gate: Arc<Gate>,
}
impl HistoryBackend for GatedReadFixture {
    fn mode(&self) -> StorageMode {
        self.inner.mode()
    }
    fn load(&self) -> LoadFuture<'_> {
        self.inner.load()
    }
    fn read_cold(&self) -> HistoryReadFuture<'_> {
        Box::pin(async move {
            let result = self.inner.read_cold().await;
            self.gate.wait().await;
            result
        })
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        self.inner.append(entries)
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        self.inner.reconcile()
    }
}
async fn finished(fixture: &Fixture) {
    let mut snapshots = fixture.session.snapshots.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !fixture.session.state.lock().unwrap().completed.is_empty() {
                return;
            }
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
fn input(text: &str) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: text.into(),
        in_reply_to: None,
    }
}
fn spawn_read(
    fixture: &Fixture,
    caller: CallerContext,
) -> tokio::task::JoinHandle<Result<SessionHistorySnapshot, Fault>> {
    let session = fixture.session.clone();
    tokio::spawn(async move { session.history_read(&caller, &read_request()).await })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_winner_serializes_load_without_blocking_other_session() {
    let read_gate = crate::backend::TestReadGate::new();
    let load_gate = Gate::new();
    let fixture = setup(|inner| {
        inner.gate_cold_read(read_gate.clone());
        Arc::new(GatedLoadFixture {
            inner,
            gate: load_gate.clone(),
        })
    });
    let before = std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap();
    let read = spawn_read(&fixture, fixture.connection.caller());
    read_gate.reached().await;
    load(&fixture);
    assert!(fixture.session.state.lock().unwrap().preparing);

    // Actual Runtime and a separate production Session/journal continue while
    // this Session's disk-only mutex is held by its content read.
    let unrelated = setup(|inner| inner);
    unrelated
        .session
        .send(&unrelated.connection.caller(), input("independent work"))
        .unwrap();
    finished(&unrelated).await;
    assert_eq!(unrelated.model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        unrelated
            .session
            .state
            .lock()
            .unwrap()
            .completed
            .values()
            .next()
            .unwrap()
            .info
            .persistence,
        PersistenceState::Persisted
    );
    assert!(!read.is_finished());
    read_gate.release();
    let page = read.await.unwrap().unwrap();
    assert_eq!(page.data.items.len(), 1);
    load_gate.reached().await;
    load_gate.release();
    settled(&fixture).await;
    let state = fixture.session.state.lock().unwrap();
    assert!(state.history_loaded);
    assert!(state.block_reason.is_none());
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
    drop(state);
    assert_eq!(
        std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap(),
        before
    );
}

#[tokio::test]
async fn delayed_read_returns_newer_owner_records_not_opening_baseline() {
    let loaded = Gate::new();
    let read_gate = Gate::new();
    let fixture = setup(|inner| {
        Arc::new(GatedReadFixture {
            inner: Arc::new(GatedLoadFixture {
                inner,
                gate: loaded.clone(),
            }),
            gate: read_gate.clone(),
        })
    });
    load(&fixture);
    loaded.reached().await;
    let read = spawn_read(&fixture, fixture.connection.caller());
    read_gate.reached().await;
    loaded.release();
    settled(&fixture).await;
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            input("accepted after the opening read"),
        )
        .unwrap();
    finished(&fixture).await;
    read_gate.release();
    let page = read.await.unwrap().unwrap();
    assert_eq!(page.data.items.len(), 3);
    assert!(
        matches!(&page.data.items[1], HistoryItemData::User { text, .. } if text == "accepted after the opening read")
    );
    assert!(matches!(
        &page.data.items[2],
        HistoryItemData::Assistant { .. }
    ));
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn delayed_cold_snapshot_yields_to_newer_retained_history() {
    let read_gate = Gate::new();
    let fixture = setup(|inner| {
        Arc::new(GatedReadFixture {
            inner,
            gate: read_gate.clone(),
        })
    });
    let read = spawn_read(&fixture, fixture.connection.caller());
    read_gate.reached().await;
    load(&fixture);
    settled(&fixture).await;
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            input("newer than cold snapshot"),
        )
        .unwrap();
    finished(&fixture).await;
    read_gate.release();
    let page = read.await.unwrap().unwrap();
    assert_eq!(page.data.items.len(), 3);
    assert!(
        matches!(&page.data.items[1], HistoryItemData::User { text, .. } if text == "newer than cold snapshot")
    );
}

#[tokio::test]
async fn timed_out_read_does_not_cancel_accepted_load() {
    let gate = Gate::new();
    let fixture = setup(|inner| {
        Arc::new(GatedLoadFixture {
            inner,
            gate: gate.clone(),
        })
    });
    load(&fixture);
    gate.reached().await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(1),
            fixture
                .session
                .history_read(&fixture.connection.caller(), &read_request())
        )
        .await
        .is_err()
    );
    assert!(fixture.session.state.lock().unwrap().preparing);
    gate.release();
    settled(&fixture).await;
    assert!(fixture.session.state.lock().unwrap().history_loaded);
    assert!(fixture.session.state.lock().unwrap().block_reason.is_none());
}

#[tokio::test]
async fn disconnect_ends_only_read_wait_before_load_publication() {
    let gate = Gate::new();
    let read_gate = Gate::new();
    let fixture = setup(|inner| {
        Arc::new(GatedReadFixture {
            inner: Arc::new(GatedLoadFixture {
                inner,
                gate: gate.clone(),
            }),
            gate: read_gate.clone(),
        })
    });
    load(&fixture);
    gate.reached().await;
    let reader = fixture.session.access.connect().unwrap();
    assert!(!reader.initial_info().can_operate);
    let caller = reader.caller();
    let request = read_request();
    let read = fixture.session.history_read(&caller, &request);
    tokio::pin!(read);
    tokio::select! {
        _ = read_gate.reached() => {}
        result = &mut read => panic!("read returned before owner publication: {result:?}"),
    }
    read_gate.release();
    assert!(futures_util::poll!(read.as_mut()).is_pending());
    reader.close();
    let fault = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(fault.kind(), Some(FaultKind::HostAccessInvalid));
    assert!(fixture.session.state.lock().unwrap().preparing);
    gate.release();
    settled(&fixture).await;
    assert!(fixture.session.state.lock().unwrap().history_loaded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_disk_reader_does_not_cancel_waiting_load() {
    let gate = crate::backend::TestReadGate::new();
    let fixture = setup(|inner| {
        inner.gate_cold_read(gate.clone());
        inner
    });
    let read = spawn_read(&fixture, fixture.connection.caller());
    gate.reached().await;
    load(&fixture);
    read.abort();
    assert!(read.await.unwrap_err().is_cancelled());
    gate.release();
    settled(&fixture).await;
    assert!(fixture.session.state.lock().unwrap().history_loaded);
    assert!(fixture.session.state.lock().unwrap().block_reason.is_none());
}

#[tokio::test]
async fn cold_snapshot_checks_source_lifetime_at_return() {
    let gate = Gate::new();
    let fixture = setup(|inner| {
        Arc::new(GatedReadFixture {
            inner,
            gate: gate.clone(),
        })
    });
    let read = spawn_read(&fixture, fixture.connection.caller());
    gate.reached().await;
    {
        let mut state = fixture.session.state.lock().unwrap();
        state.reclaimed = true;
        fixture.session.publish(&mut state);
    }
    gate.release();
    assert_eq!(
        read.await.unwrap().unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
}

#[tokio::test]
async fn cold_read_is_read_only_and_preserves_pause_and_queued_input() {
    let fixture = setup(|inner| inner);
    fixture
        .session
        .control(
            &fixture.connection.caller(),
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            SessionControlKind::Pause,
        )
        .unwrap();
    fixture
        .session
        .send(&fixture.connection.caller(), input("must remain queued"))
        .unwrap();
    let before = fixture
        .session
        .snapshot(&fixture.connection.caller())
        .unwrap();
    let bytes = std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap();
    let reader = fixture.session.access.connect().unwrap();
    assert!(!reader.initial_info().can_operate);
    let page = fixture
        .session
        .history_read(&reader.caller(), &read_request())
        .await
        .unwrap();
    assert_eq!(page.data.items.len(), 1);
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(
            fixture
                .session
                .snapshot(&fixture.connection.caller())
                .unwrap()
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap(),
        bytes
    );
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.session.state.lock().unwrap().history_loaded);
    assert!(
        fixture
            .session
            .state
            .lock()
            .unwrap()
            .readable_history
            .is_none()
    );
}

#[tokio::test]
async fn cold_pages_keep_public_positions_and_input_provenance_across_noncontent_records() {
    use minicore_runtime::{
        AssistantHistory, UserHistory, UserMessageKind,
        model::{AssistantPart, ModelPartial, Usage},
    };

    let fixture = setup(|inner| inner);
    let turn_id = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let message_id = MessageId::new().unwrap();
    let reply = MessageId::new().unwrap();
    // Explicit saved-record fixture spanning every public content variant.
    let entries = vec![
        HistoryEntry::Input {
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
            source: MessageSource::User,
            mode: MessageMode::Steer,
            in_reply_to: Some(reply.clone()),
            user: UserHistory {
                loop_id,
                kind: UserMessageKind::Steering,
                input: UserInput::text("original input 原文").unwrap(),
            },
        },
        HistoryEntry::Runtime {
            turn_id: turn_id.clone(),
            response_index: 0,
            item: HistoryItem::Assistant(AssistantHistory {
                loop_id,
                request_index: 0,
                model: "test/model".parse().unwrap(),
                reasoning: ReasoningPreference::Auto,
                content: vec![AssistantPart::Text("complete response".into())],
                provider_replay: None,
                finish_reason: ModelFinishReason::Stop,
                usage: Usage::default(),
            }),
        },
        HistoryEntry::Partial {
            turn_id: turn_id.clone(),
            loop_id,
            response_index: 1,
            partial: ModelPartial {
                request_index: 1,
                content: vec![AssistantPart::Text("unfinished response".into())],
                incomplete_tool_calls: Vec::new(),
                usage: None,
                provider_replay: None,
            },
        },
        HistoryEntry::TurnFinished {
            turn_id,
            outcome: TurnOutcome::Cancelled,
            reason: Some("saved-record fixture".into()),
        },
        summary("literal summary"),
        HistoryEntry::Compaction {
            source_position: 6,
            covered_items: 1,
            summary: SummaryHistory {
                content: Text::new("compacted seed"),
            },
        },
        HistoryEntry::TurnFinished {
            turn_id: TurnId::new().unwrap(),
            outcome: TurnOutcome::Completed,
            reason: None,
        },
    ];
    let mut journal = fixture.store.open_journal().unwrap();
    journal.accept(entries.clone()).unwrap();
    journal.persist_pending().unwrap();
    drop(journal);
    let expected = std::iter::once(summary("persisted seed"))
        .chain(entries)
        .filter_map(|entry| HistoryItemData::from_entry(&entry))
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 6);
    assert!(matches!(
        &expected[1],
        HistoryItemData::User {
            text, message_id: Some(id), source: Some(MessageSource::User),
            mode: Some(MessageMode::Steer), in_reply_to: Some(actual_reply),
        } if text == "original input 原文" && id == &message_id && actual_reply == &reply
    ));
    for limit in [1, 2, 3, 100] {
        let mut cursor = None;
        let mut actual = Vec::new();
        loop {
            let page = fixture
                .session
                .history_read(
                    &fixture.connection.caller(),
                    &HistoryReadRequest { cursor, limit },
                )
                .await
                .unwrap();
            actual.extend(page.data.items);
            cursor = page.data.next_cursor;
            if cursor.is_none() {
                break;
            }
            assert_eq!(
                cursor,
                Some(format!(
                    "v1.{}.{}.{}",
                    fixture.session.incarnation,
                    fixture.session.id,
                    actual.len()
                ))
            );
        }
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
    }
    for position in [expected.len(), expected.len() + 1] {
        let result = fixture
            .session
            .history_read(
                &fixture.connection.caller(),
                &HistoryReadRequest {
                    cursor: Some(format!(
                        "v1.{}.{}.{position}",
                        fixture.session.incarnation, fixture.session.id
                    )),
                    limit: 1,
                },
            )
            .await;
        if position == expected.len() {
            let page = result.unwrap();
            assert!(page.data.items.is_empty());
            assert!(page.data.next_cursor.is_none());
        } else {
            assert_eq!(
                result.unwrap_err().kind(),
                Some(FaultKind::SessionHistoryCursorInvalid)
            );
        }
    }
    assert!(!fixture.session.state.lock().unwrap().history_loaded);
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn corrupt_and_partial_cold_histories_fail_without_repair_or_prefix() {
    use std::io::Write;
    for tail in [b"{broken}\n".as_slice(), b"{\"schema_version\":".as_slice()] {
        let fixture = setup(|inner| inner);
        let path = fixture.store.canonical_path().join("history.jsonl");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(tail)
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let before = fixture
            .session
            .snapshot(&fixture.connection.caller())
            .unwrap();
        let fault = fixture
            .session
            .history_read(
                &fixture.connection.caller(),
                &HistoryReadRequest {
                    cursor: None,
                    limit: 1,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(fault.kind(), Some(FaultKind::SessionHistoryLoadFailed));
        let encoded = serde_json::to_value(&fault).unwrap();
        assert!(encoded.to_string().contains("byte_offset"));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(
                fixture
                    .session
                    .snapshot(&fixture.connection.caller())
                    .unwrap()
            )
            .unwrap()
        );
        assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
    }
}

struct InternalReaderFixture;
impl crate::control::InternalAuthority for InternalReaderFixture {
    fn check(&self) -> Result<(), Fault> {
        Ok(())
    }
    fn source(&self) -> MessageSource {
        MessageSource::User
    }
}
#[tokio::test]
async fn internal_cold_reader_does_not_require_external_connection_identity() {
    let fixture = setup(|inner| inner);
    let caller = CallerContext::internal(&fixture.session.access, Arc::new(InternalReaderFixture));
    let page = fixture
        .session
        .history_read(&caller, &read_request())
        .await
        .unwrap();
    assert_eq!(page.data.items.len(), 1);
}

struct UnsupportedReadFixture(Arc<AtomicUsize>);
impl HistoryBackend for UnsupportedReadFixture {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Vec::new()) })
    }
    fn append<'a>(&'a self, _: Vec<HistoryEntry>) -> SaveFuture<'a> {
        panic!("read must not append")
    }
}
#[tokio::test]
async fn unsupported_custom_read_never_falls_back_to_mutating_load() {
    let loads = Arc::new(AtomicUsize::new(0));
    let fixture = setup(|_| Arc::new(UnsupportedReadFixture(loads.clone())));
    let fault = fixture
        .session
        .history_read(&fixture.connection.caller(), &read_request())
        .await
        .unwrap_err();
    assert_eq!(fault.kind(), Some(FaultKind::SessionHistoryLoadFailed));
    assert_eq!(loads.load(Ordering::SeqCst), 0);
}

struct GatedResourcesFixture {
    inner: Arc<dyn ResourceResolver>,
    gate: Arc<Gate>,
}
impl ResourceResolver for GatedResourcesFixture {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.gate.wait().await;
            self.inner.resolve(config).await
        })
    }
}
#[tokio::test]
async fn loaded_history_publication_does_not_wait_for_runtime_resource_preparation() {
    let loaded = Gate::new();
    let read_gate = Gate::new();
    let preparing = Gate::new();
    let fixture = setup(|inner| {
        Arc::new(GatedReadFixture {
            inner: Arc::new(GatedLoadFixture {
                inner,
                gate: loaded.clone(),
            }),
            gate: read_gate.clone(),
        })
    });
    {
        let mut resources = fixture.session.resources.lock().unwrap();
        let inner = resources.take().unwrap();
        *resources = Some(Arc::new(GatedResourcesFixture {
            inner,
            gate: preparing.clone(),
        }));
    }
    fixture
        .session
        .send(&fixture.connection.caller(), input("load then prepare"))
        .unwrap();
    loaded.reached().await;
    let caller = fixture.connection.caller();
    let request = read_request();
    let read = fixture.session.history_read(&caller, &request);
    tokio::pin!(read);
    tokio::select! {
        _ = read_gate.reached() => {}
        result = &mut read => panic!("read returned before owner publication: {result:?}"),
    }
    read_gate.release();
    assert!(futures_util::poll!(read.as_mut()).is_pending());
    loaded.release();
    preparing.reached().await;
    let page = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.data.items.len(), 1);
    assert!(fixture.session.state.lock().unwrap().preparing);
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
    preparing.release();
    finished(&fixture).await;
}

#[tokio::test]
async fn delayed_cold_read_preserves_newer_failed_original_save() {
    let gate = Gate::new();
    let mut backend = None;
    let fixture = setup(|inner| {
        backend = Some(inner.clone());
        Arc::new(GatedReadFixture {
            inner,
            gate: gate.clone(),
        })
    });
    let backend = backend.unwrap();
    let path = fixture.store.canonical_path().join("history.jsonl");
    let bytes = std::fs::read(&path).unwrap();
    let read = spawn_read(&fixture, fixture.connection.caller());
    gate.reached().await;
    load(&fixture);
    settled(&fixture).await;
    backend.inject_next_append_fault(crate::backend::TestAppendFault::NoWrite);
    let mut snapshots = fixture.session.snapshots.subscribe();
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            input("accepted but unconfirmed"),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if fixture
                .session
                .state
                .lock()
                .unwrap()
                .history_work
                .as_ref()
                .is_some_and(|work| work.fault.is_some())
            {
                return;
            }
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    gate.release();
    let page = read.await.unwrap().unwrap();
    assert_eq!(page.data.items.len(), 3);
    {
        let state = fixture.session.state.lock().unwrap();
        assert_eq!(
            state.active.as_ref().unwrap().info.persistence,
            PersistenceState::Failed
        );
        assert!(state.history_work.is_some());
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 1);
    // The read did not continue or replace the original append. Only this
    // explicit owner command may reconcile it, without another model call.
    let mut snapshots = fixture.session.snapshots.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !fixture.session.state.lock().unwrap().driver_running {
                break;
            }
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    finished(&fixture).await;
    assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 1);
    let retained = fixture
        .session
        .history_read(&fixture.connection.caller(), &read_request())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(retained.data).unwrap(),
        serde_json::to_value(page.data).unwrap()
    );
}

#[tokio::test]
async fn absent_cold_history_fails_without_creating_a_journal() {
    let fixture = setup(|inner| inner);
    let path = fixture.store.canonical_path().join("history.jsonl");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        fixture
            .session
            .history_read(&fixture.connection.caller(), &read_request())
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionHistoryLoadFailed)
    );
    assert!(!path.exists());
    assert!(!fixture.session.state.lock().unwrap().history_loaded);
}

#[tokio::test]
async fn complete_cold_last_line_without_newline_is_accepted_unchanged() {
    let fixture = setup(|inner| inner);
    let path = fixture.store.canonical_path().join("history.jsonl");
    let mut bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'));
    std::fs::write(&path, &bytes).unwrap();
    let page = fixture
        .session
        .history_read(&fixture.connection.caller(), &read_request())
        .await
        .unwrap();
    assert_eq!(page.data.items.len(), 1);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

struct MemoryReadFixture;
impl HistoryBackend for MemoryReadFixture {
    fn mode(&self) -> StorageMode {
        StorageMode::Memory
    }
    fn load(&self) -> LoadFuture<'_> {
        panic!("memory read must not load")
    }
    fn read_cold(&self) -> HistoryReadFuture<'_> {
        panic!("memory read must not enter disk fallback")
    }
    fn append<'a>(&'a self, _: Vec<HistoryEntry>) -> SaveFuture<'a> {
        panic!("memory read must not append")
    }
}
#[tokio::test]
async fn memory_retained_read_never_enters_a_storage_backend() {
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let (resources, model) = resources();
    let session = SessionController::new(
        SessionId::new().unwrap(),
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        SessionConfig::default(),
        access,
        resources,
        Arc::new(MemoryReadFixture),
        vec![HistoryItem::Summary(SummaryHistory {
            content: Text::new("memory only"),
        })],
        Arc::new(AtomicBool::new(true)),
    );
    let page = session
        .history_read(&connection.caller(), &read_request())
        .await
        .unwrap();
    assert!(
        matches!(&page.data.items[..], [HistoryItemData::Summary { text }] if text == "memory only")
    );
    assert!(session.store.lock().unwrap().is_none());
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

fn control_request() -> SessionControlRequest {
    SessionControlRequest {
        operation_id: OperationId::new().unwrap(),
    }
}
#[tokio::test]
async fn real_host_cold_read_load_and_runtime_preserve_read_only_boundaries() {
    let fixture = setup(|inner| inner);
    let target = SessionRef {
        agent_id: fixture.session.agent_id.clone(),
        session_id: fixture.session.id.clone(),
    };
    let path = fixture.store.canonical_path().join("history.jsonl");
    let Fixture {
        _dir: dir,
        store,
        session,
        connection,
        model: _,
    } = fixture;
    drop(session);
    drop(connection);
    drop(store);
    let (resources, model) = resources();
    let host = crate::Host::open_persistent(
        dir.path(),
        HostSettings {
            name: "task052 Host".into(),
        },
        resources,
    )
    .await
    .unwrap();
    let operator = host.connect().unwrap();
    let reader = host.connect().unwrap();
    let caller = operator.caller();
    assert!(!reader.initial_info().can_operate);
    host.session_pause(&caller, &target, control_request())
        .unwrap();
    host.send(
        &caller,
        SendMessage::new(input("queued until explicit resume"), target.clone()),
    )
    .unwrap();
    let before = host.session_state(&caller, &target).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let page = host
        .history_read(&reader.caller(), &target, &read_request())
        .await
        .unwrap();
    assert_eq!(page.data.items.len(), 1);
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(host.session_state(&caller, &target).unwrap()).unwrap()
    );
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        host.load_session(&reader.caller(), &target)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    let mut states = host.session_states(&caller, &target).unwrap();
    host.load_session(&caller, &target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while states.snapshot().unwrap().data.preparing {
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let loaded = states.snapshot().unwrap();
    assert_eq!(loaded.data.residency, Residency::Loaded);
    assert_eq!(loaded.data.next_turn.len(), 1);
    assert!(loaded.data.block_reason.is_none());
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    host.session_resume(&caller, &target, control_request())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while states.snapshot().unwrap().data.last_result.is_none() {
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let turn = host.turns(&caller, &target).unwrap().pop().unwrap();
    assert_eq!(turn.persistence, PersistenceState::Persisted);
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    let latest = host
        .history_read(&reader.caller(), &target, &read_request())
        .await
        .unwrap();
    assert_eq!(latest.data.items.len(), 3);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.info(&caller).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(host.info(&caller).unwrap().shutdown_fault.is_none());
}
