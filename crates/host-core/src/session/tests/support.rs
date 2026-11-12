//! Named model, tool and journal fixtures for Session owner scenarios.
use super::*;
use minicore_host_storage::testing::read_history_text;

pub(super) struct FixtureModel {
    pub(super) descriptor: ModelDescriptor,
    pub(super) calls: AtomicUsize,
    pub(super) scripts: Mutex<VecDeque<Vec<ModelEvent>>>,
}
impl Model for FixtureModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let events = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .expect("declared fixture response");
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::iter(events.into_iter().map(Ok))) as ModelStream)
        })
    }
}
pub(super) struct CountTool {
    pub(super) spec: ToolSpec,
    pub(super) calls: Arc<AtomicUsize>,
}
impl Tool for CountTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("actual fixture tool result").unwrap(),
            ))
        })
    }
}
pub(super) struct Resources {
    pub(super) model: Arc<dyn Model>,
    pub(super) tools: ToolSet,
}
impl ResourceResolver for Resources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|e| Fault::new(FaultKind::SessionConfigInvalid, e.to_string()))
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    self.tools.clone(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
pub(in crate::session) struct Fixture {
    pub(super) _dir: tempfile::TempDir,
    pub(super) store: SessionStore,
    pub(super) access: Arc<ConnectionRegistry>,
    pub(super) connection: crate::ConnectionHandle,
    pub(in crate::session) session: Arc<SessionController>,
}
pub(super) fn model(scripts: Vec<Vec<ModelEvent>>) -> Arc<FixtureModel> {
    Arc::new(FixtureModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128_000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        calls: AtomicUsize::new(0),
        scripts: Mutex::new(scripts.into()),
    })
}
pub(super) fn final_events(text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::text_delta(text),
        ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        },
    ]
}
pub(super) fn setup(
    resources: Arc<dyn ResourceResolver>,
    wrap: impl FnOnce(Arc<JournalHistory>) -> Arc<dyn HistoryBackend>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(dir.path().join("host")).unwrap();
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
                name: "fixture".into(),
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
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let backend = wrap(Arc::new(JournalHistory::new(store.clone())));
    let session = SessionController::new(
        id,
        agent_id,
        host_id,
        incarnation,
        config,
        access.clone(),
        resources,
        backend,
        Vec::new(),
        Arc::new(AtomicBool::new(true)),
    );
    Fixture {
        _dir: dir,
        store,
        access,
        connection,
        session,
    }
}
pub(super) fn send(fixture: &Fixture, text: &str) -> MessageAccepted {
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: text.into(),
                in_reply_to: None,
            },
        )
        .unwrap()
}
pub(super) async fn until(fixture: &Fixture, predicate: impl Fn(&SessionState) -> bool) {
    let mut changed = fixture.session.snapshots.subscribe();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if predicate(&fixture.session.state.lock().unwrap()) {
                return;
            }
            changed.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
pub(super) fn records(fixture: &Fixture) -> Vec<HistoryRecord> {
    read_history_text(fixture.store.canonical_path().join("history.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
pub(super) struct GatedJournal {
    pub(super) inner: Arc<JournalHistory>,
    pub(super) gates: Mutex<VecDeque<Arc<Semaphore>>>,
    pub(super) entered: mpsc::UnboundedSender<usize>,
    pub(super) count: AtomicUsize,
}
impl HistoryBackend for GatedJournal {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.inner.load()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        Box::pin(async move {
            let number = self.count.fetch_add(1, Ordering::SeqCst);
            let gate = self.gates.lock().unwrap().pop_front();
            self.entered.send(number).unwrap();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            self.inner.append(entries).await
        })
    }
}

pub(super) struct FailedSave {
    pub(super) count: AtomicUsize,
    pub(super) inner: Arc<JournalHistory>,
}
impl HistoryBackend for FailedSave {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.inner.load()
    }
    fn append<'a>(&'a self, _: Vec<HistoryEntry>) -> SaveFuture<'a> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "injected save failure",
            ))
        })
    }
}

pub(super) struct FailureJournal {
    pub(super) inner: Arc<JournalHistory>,
    pub(super) failures: Mutex<VecDeque<Option<crate::backend::TestAppendFault>>>,
    pub(super) appends: AtomicUsize,
    pub(super) reconciles: AtomicUsize,
    pub(super) reconcile_gate: Option<Arc<Semaphore>>,
    pub(super) reconcile_entered: mpsc::UnboundedSender<()>,
}
impl HistoryBackend for FailureJournal {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.inner.load()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        self.appends.fetch_add(1, Ordering::SeqCst);
        if let Some(failure) = self.failures.lock().unwrap().pop_front().flatten() {
            self.inner.inject_next_append_fault(failure);
        }
        self.inner.append(entries)
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        Box::pin(async move {
            self.reconciles.fetch_add(1, Ordering::SeqCst);
            let _ = self.reconcile_entered.send(());
            if let Some(gate) = &self.reconcile_gate {
                gate.acquire().await.unwrap().forget();
            }
            self.inner.reconcile().await
        })
    }
    fn release(&self) -> Pin<Box<dyn Future<Output = Result<(), Fault>> + Send + '_>> {
        self.inner.release()
    }
}
pub(super) fn failure_setup(
    model: Arc<dyn Model>,
    tools: ToolSet,
    failures: Vec<Option<crate::backend::TestAppendFault>>,
    gate: Option<Arc<Semaphore>>,
) -> (Fixture, Arc<FailureJournal>, mpsc::UnboundedReceiver<()>) {
    let (entered, received) = mpsc::unbounded_channel();
    let captured = Arc::new(Mutex::new(None));
    let out = captured.clone();
    let fixture = setup(Arc::new(Resources { model, tools }), |inner| {
        let backend = Arc::new(FailureJournal {
            inner,
            failures: Mutex::new(failures.into()),
            appends: AtomicUsize::new(0),
            reconciles: AtomicUsize::new(0),
            reconcile_gate: gate,
            reconcile_entered: entered,
        });
        *out.lock().unwrap() = Some(backend.clone());
        backend
    });
    let backend = captured.lock().unwrap().take().unwrap();
    (fixture, backend, received)
}
pub(super) async fn blocked(fixture: &Fixture) -> TurnInfo {
    until(fixture, |state| {
        !state.driver_running
            && state.history_work.is_some()
            && !state.history_work.as_ref().is_some_and(|work| work.running)
            && state
                .active
                .as_ref()
                .is_some_and(|active| active.info.state == TurnState::Finalizing)
    })
    .await;
    fixture
        .session
        .state
        .lock()
        .unwrap()
        .active
        .as_ref()
        .unwrap()
        .info
        .clone()
}
pub(super) fn assert_single_terminal(fixture: &Fixture, outcome: TurnOutcome, partials: usize) {
    let saved = records(fixture);
    assert_eq!(
        saved
            .iter()
            .filter(|record| matches!(record.entry, HistoryEntry::Input { .. }))
            .count(),
        1
    );
    assert_eq!(
        saved
            .iter()
            .filter(|record| matches!(record.entry, HistoryEntry::Partial { .. }))
            .count(),
        partials
    );
    assert_eq!(
        saved
            .iter()
            .filter(|record| matches!(record.entry, HistoryEntry::TurnFinished { .. }))
            .count(),
        1
    );
    assert!(
        matches!(saved.last().unwrap().entry, HistoryEntry::TurnFinished { outcome: saved, .. } if saved == outcome)
    );
    let state = fixture.session.state.lock().unwrap();
    assert_eq!(state.completed.len(), 1);
    let result = state.completed.values().next().unwrap();
    assert_eq!(result.info.outcome, Some(outcome));
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert!(result.info.persistence_fault.is_none());
    assert_eq!(result.partials.len(), partials);
    assert!(state.history_work.is_none());
    assert!(state.block_reason.is_none());
}

pub(super) struct PartialUntilCancelled {
    pub(super) descriptor: minicore_host_runtime::model::ModelDescriptor,
    pub(super) consumed_delta: Arc<Semaphore>,
}
impl Model for PartialUntilCancelled {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        use futures_util::StreamExt;
        let consumed = self.consumed_delta.clone();
        Box::pin(async move {
            let mut signal = Some(consumed);
            let pending = futures_util::stream::poll_fn(move |_| {
                if let Some(signal) = signal.take() {
                    signal.add_permits(1);
                }
                std::task::Poll::Pending
            });
            Ok(Box::pin(
                futures_util::stream::iter([Ok(ModelEvent::text_delta(
                    "actual cancellation partial",
                ))])
                .chain(pending),
            ) as ModelStream)
        })
    }
}

pub(in crate::session) async fn original_save_fixture() -> Fixture {
    let (fixture, _, _) = failure_setup(
        model(vec![final_events("binding fixture")]),
        ToolSet::default(),
        vec![Some(crate::backend::TestAppendFault::NoWrite)],
        None,
    );
    send(&fixture, "original binding");
    blocked(&fixture).await;
    fixture
}

pub(super) struct ConstructorPanicHistory;
impl HistoryBackend for ConstructorPanicHistory {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn append<'a>(&'a self, _: Vec<HistoryEntry>) -> SaveFuture<'a> {
        panic!("fixture append construction panic")
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        panic!("fixture reconciliation construction panic")
    }
}

pub(super) async fn until_releasing(session: &Arc<SessionController>) {
    // The state claim is the deterministic gate; yields do not guess elapsed I/O time.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !session.state.lock().unwrap().releasing {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

/// Holds the first real journal save at a known Runtime/Session handoff, and
/// optionally makes that original append fail without discarding its range.
pub(super) struct TerminalClaimJournal {
    pub(super) inner: Arc<JournalHistory>,
    pub(super) entered: Arc<Semaphore>,
    pub(super) release: Arc<Semaphore>,
    pub(super) first: AtomicBool,
    pub(super) fail_first: bool,
}
impl HistoryBackend for TerminalClaimJournal {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.inner.load()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        Box::pin(async move {
            if (self.fail_first
                || entries
                    .iter()
                    .any(|entry| matches!(entry, HistoryEntry::Partial { .. })))
                && self.first.swap(false, Ordering::SeqCst)
            {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
                if self.fail_first {
                    self.inner
                        .inject_next_append_fault(crate::backend::TestAppendFault::NoWrite);
                }
            }
            self.inner.append(entries).await
        })
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        self.inner.reconcile()
    }
}
pub(super) fn terminal_claim_fixture(
    events: Vec<ModelEvent>,
    fail_first: bool,
) -> (Fixture, Arc<Semaphore>, Arc<Semaphore>) {
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let fixture = setup(
        Arc::new(Resources {
            model: model(vec![events]),
            tools: ToolSet::default(),
        }),
        |inner| {
            Arc::new(TerminalClaimJournal {
                inner,
                entered: entered.clone(),
                release: release.clone(),
                first: AtomicBool::new(true),
                fail_first,
            })
        },
    );
    (fixture, entered, release)
}

pub(super) struct CancelDropPanicModel {
    pub(super) descriptor: ModelDescriptor,
    pub(super) entered: Arc<Semaphore>,
}
pub(super) struct CancelDropPanicStream(Option<Arc<Semaphore>>);
impl futures_util::Stream for CancelDropPanicStream {
    type Item = Result<ModelEvent, minicore_host_runtime::model::ModelError>;
    fn poll_next(
        mut self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let Some(entered) = self.0.take() {
            entered.add_permits(1);
        }
        std::task::Poll::Pending
    }
}
impl Drop for CancelDropPanicStream {
    fn drop(&mut self) {
        panic!("explicit model stream destructor panic after cancellation");
    }
}
impl Model for CancelDropPanicModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let entered = self.entered.clone();
        Box::pin(async { Ok(Box::pin(CancelDropPanicStream(Some(entered))) as ModelStream) })
    }
}
