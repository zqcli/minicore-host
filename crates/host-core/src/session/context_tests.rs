//! Deterministic save-boundary regressions at the Session owner. Production Host
//! routing is exercised separately in tests/session_context.rs. The gated
//! backend wraps the real journal, including its original-range reconciliation.
use super::*;
use crate::backend::{JournalHistory, TestAppendFault};
use minicore_host_runtime::{
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelMessage,
        ModelRequest, ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
        Usage,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use minicore_host_storage::testing::read_history_bytes;
use minicore_host_storage::{AgentData, AgentRecord, DataRoot, HostRecord, SessionData};
use std::{collections::BTreeSet, sync::atomic::AtomicUsize, time::Duration};
use tokio::sync::Semaphore;

struct ScriptedModel {
    descriptor: ModelDescriptor,
    ordinary: AtomicUsize,
    utility: AtomicUsize,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let utility = matches!(request.messages().first(), Some(ModelMessage::System(text))
            if text.starts_with("Summarize historical conversation data"));
        let text = if utility {
            self.utility.fetch_add(1, Ordering::SeqCst);
            "Committed scripted summary of prior tasks.".to_owned()
        } else {
            self.ordinary.fetch_add(1, Ordering::SeqCst);
            "long ordinary scripted answer ".repeat(100)
        };
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::iter([
                Ok(ModelEvent::text_delta(text)),
                Ok(ModelEvent::Usage {
                    usage: Usage::from_optional(Some(if utility { 123 } else { 44 }), None, None),
                }),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
struct Resources(Arc<ScriptedModel>);
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.0.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
struct GatedJournal {
    inner: Arc<JournalHistory>,
    armed: AtomicBool,
    entered: Semaphore,
    release: Semaphore,
    loads: AtomicUsize,
    appends: AtomicUsize,
    reconciles: AtomicUsize,
}
impl HistoryBackend for GatedJournal {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        self.appends.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.armed.swap(false, Ordering::SeqCst) {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
                self.inner
                    .inject_next_append_fault(TestAppendFault::FullWrite);
            }
            self.inner.append(entries).await
        })
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        self.reconciles.fetch_add(1, Ordering::SeqCst);
        self.inner.reconcile()
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    connection: crate::ConnectionHandle,
    session: Arc<SessionController>,
    backend: Arc<GatedJournal>,
    model: Arc<ScriptedModel>,
    store: SessionStore,
}
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(directory.path()).unwrap();
    let host_id = HostId::new().unwrap();
    let host = root.host(host_id.clone());
    host.save_host(&HostRecord::new(
        host_id.clone(),
        crate::HostConfig::default(),
        0,
    ))
    .unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let agent_id = AgentId::new().unwrap();
    let agent = host.agent(agent_id.clone(), StorageMode::Persistent);
    let mut config = AgentConfig::default();
    config.session_defaults.model = "test/model".parse().unwrap();
    config.session_defaults.execution.compaction = CompactionSettings {
        enabled: false,
        trigger_percent: 50,
        target_percent: 20,
    };
    agent
        .save(&AgentRecord::new(
            agent_id.clone(),
            host_id.clone(),
            AgentData {
                name: "gated save fixture".into(),
                mode: StorageMode::Persistent,
                config: config.clone(),
                config_revision: 0,
                default_session_id: None,
            },
        ))
        .unwrap();
    let id = SessionId::new().unwrap();
    let store = agent.session(id.clone(), StorageMode::Persistent).unwrap();
    let record = SessionRecord::new(
        id,
        agent_id,
        SessionData {
            title: None,
            mode: StorageMode::Persistent,
            agent_config_revision: 0,
            config_revision: 0,
            effective_config: SessionConfig::from_agent(&config, &SessionOverrides::default())
                .unwrap(),
            parent: None,
            closed: None,
        },
    );
    store.save(&record).unwrap();
    let model = Arc::new(ScriptedModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            4096,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        ordinary: AtomicUsize::new(0),
        utility: AtomicUsize::new(0),
    });
    let backend = Arc::new(GatedJournal {
        inner: Arc::new(JournalHistory::new(store.clone())),
        armed: AtomicBool::new(false),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
        loads: AtomicUsize::new(0),
        appends: AtomicUsize::new(0),
        reconciles: AtomicUsize::new(0),
    });
    let mut session = SessionController::from_record(
        record,
        store.clone(),
        host_id,
        incarnation,
        access,
        Arc::new(Resources(model.clone())),
        Arc::new(AtomicBool::new(true)),
        false, // This fixture has just created the durable record.
    );
    // Fixture construction only, before publication and before any execution.
    Arc::get_mut(&mut session).unwrap().backend = backend.clone();
    session.publish_initial();
    Fixture {
        _directory: directory,
        connection,
        session,
        backend,
        model,
        store,
    }
}
fn send(fixture: &Fixture) {
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "ordinary task details ".repeat(100),
                in_reply_to: None,
            },
        )
        .unwrap();
}
async fn read(fixture: &Fixture) -> SessionContextSnapshot {
    fixture
        .session
        .context(&fixture.connection.caller())
        .await
        .unwrap()
}
async fn until(
    fixture: &Fixture,
    condition: impl Fn(&SessionContextSnapshot) -> bool,
) -> SessionContextSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = read(fixture).await;
            if condition(&snapshot) {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
async fn gate(fixture: &Fixture) {
    tokio::time::timeout(Duration::from_secs(5), fixture.backend.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
}

#[tokio::test]
async fn pending_and_failed_turn_saves_are_distinct_from_readable_reported_usage() {
    let fixture = fixture();
    fixture.backend.armed.store(true, Ordering::SeqCst);
    send(&fixture);
    gate(&fixture).await;
    let pending = read(&fixture).await;
    assert!(pending.data.history_save.pending && pending.data.history_save.running);
    assert!(pending.data.history_save.fault.is_none());
    assert_eq!(pending.data.history.as_ref().unwrap().raw_items, 2);
    assert_eq!(
        pending
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .as_ref()
            .unwrap()
            .usage
            .input_tokens(),
        Some(44)
    );
    fixture.backend.release.add_permits(1);
    let failed = until(&fixture, |context| {
        context.data.history_save.fault.is_some()
    })
    .await;
    assert!(failed.data.history_save.pending && !failed.data.history_save.running);
    assert_eq!(
        failed.data.history_save.fault.unwrap().kind(),
        Some(FaultKind::SessionHistoryWriteFailed)
    );
    let bytes = read_history_bytes(fixture.store.canonical_path().join("history.jsonl")).unwrap();
    assert!(
        !bytes.is_empty(),
        "readable bytes are not a synchronization confirmation"
    );
    for _ in 0..10 {
        let context = read(&fixture).await;
        assert_eq!(context.data.history.as_ref().unwrap().raw_items, 2);
        assert_eq!(
            context
                .data
                .history
                .as_ref()
                .unwrap()
                .latest_response
                .as_ref()
                .unwrap()
                .usage
                .input_tokens(),
            Some(44)
        );
        assert!(context.data.history_save.fault.is_some());
    }
    assert_eq!(fixture.backend.loads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.backend.appends.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.backend.reconciles.load(Ordering::SeqCst), 0);
    assert_eq!(
        read_history_bytes(fixture.store.canonical_path().join("history.jsonl")).unwrap(),
        bytes
    );
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    let settled = until(&fixture, |context| context.data.active_turn.is_none()).await;
    assert!(!settled.data.history_save.pending);
    assert!(settled.data.history_save.fault.is_none());
    assert_eq!(settled.data.history.as_ref().unwrap().raw_items, 2);
    assert_eq!(fixture.backend.reconciles.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.model.ordinary.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn compaction_candidate_is_not_a_committed_projection_during_pending_or_failed_save() {
    let fixture = fixture();
    for count in 1..=3 {
        send(&fixture);
        until(&fixture, |context| {
            context.data.active_turn.is_none()
                && context
                    .data
                    .history
                    .as_ref()
                    .is_some_and(|history| history.raw_items == count * 2)
        })
        .await;
    }
    fixture.session.pause(&fixture.connection.caller()).unwrap();
    fixture.backend.armed.store(true, Ordering::SeqCst);
    fixture
        .session
        .compact(
            &fixture.connection.caller(),
            CompactRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    gate(&fixture).await;
    let pending = read(&fixture).await;
    assert_eq!(pending.data.history.as_ref().unwrap().raw_items, 6);
    assert!(pending.data.history.as_ref().unwrap().projection.is_none());
    assert!(pending.data.history_save.pending && pending.data.history_save.running);
    assert!(matches!(
        pending.data.compaction.unwrap().state,
        CompactionState::Committing {
            persistence: PersistenceState::Pending,
            ..
        }
    ));
    fixture.backend.release.add_permits(1);
    let failed = until(&fixture, |context| {
        context.data.history_save.fault.is_some()
    })
    .await;
    assert!(failed.data.history.as_ref().unwrap().projection.is_none());
    assert!(matches!(
        failed.data.compaction.unwrap().state,
        CompactionState::Committing {
            persistence: PersistenceState::Failed,
            fault: Some(_),
            ..
        }
    ));
    let utilities = fixture.model.utility.load(Ordering::SeqCst);
    for _ in 0..10 {
        assert!(
            read(&fixture)
                .await
                .data
                .history
                .as_ref()
                .unwrap()
                .projection
                .is_none()
        );
    }
    assert_eq!(fixture.backend.reconciles.load(Ordering::SeqCst), 0);
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    let committed = until(&fixture, |context| {
        context.data.history.as_ref().unwrap().projection.is_some()
    })
    .await;
    assert!(!committed.data.history_save.pending);
    assert_eq!(committed.data.history.as_ref().unwrap().raw_items, 6);
    assert!(
        committed
            .data
            .history
            .as_ref()
            .unwrap()
            .projection
            .as_ref()
            .unwrap()
            .summary
            .content
            .as_str()
            .contains("Committed scripted summary")
    );
    assert_eq!(
        committed
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .as_ref()
            .unwrap()
            .usage
            .input_tokens(),
        Some(44)
    );
    assert_eq!(fixture.model.utility.load(Ordering::SeqCst), utilities);
    assert_eq!(fixture.model.ordinary.load(Ordering::SeqCst), 3);
}
