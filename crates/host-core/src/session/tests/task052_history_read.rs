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
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        self.inner.append(entries)
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
    host.save_host(&HostRecord::new(host_id.clone(), (), 0))
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
