use super::*;
use crate::backend::JournalHistory;
use minicore_host_storage::{
    AgentData, AgentRecord, DataRoot, HistoryRecord, HostRecord, SessionData, SessionRecord,
    SessionStore,
};
use minicore_runtime::{
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet,
        ToolSpec,
    },
};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Semaphore, mpsc};

struct FixtureModel {
    descriptor: ModelDescriptor,
    calls: AtomicUsize,
    scripts: Mutex<VecDeque<Vec<ModelEvent>>>,
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
struct CountTool {
    spec: ToolSpec,
    calls: Arc<AtomicUsize>,
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
struct Resources {
    model: Arc<dyn Model>,
    tools: ToolSet,
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
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
pub(super) struct Fixture {
    _dir: tempfile::TempDir,
    store: SessionStore,
    access: Arc<ConnectionRegistry>,
    connection: crate::ConnectionHandle,
    pub(super) session: Arc<SessionController>,
}
fn model(scripts: Vec<Vec<ModelEvent>>) -> Arc<FixtureModel> {
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
fn final_events(text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::text_delta(text),
        ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        },
    ]
}
fn setup(
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
fn send(fixture: &Fixture, text: &str) -> MessageAccepted {
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
async fn until(fixture: &Fixture, predicate: impl Fn(&SessionState) -> bool) {
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
fn records(fixture: &Fixture) -> Vec<HistoryRecord> {
    std::fs::read_to_string(fixture.store.canonical_path().join("history.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
struct GatedJournal {
    inner: Arc<JournalHistory>,
    gates: Mutex<VecDeque<Arc<Semaphore>>>,
    entered: mpsc::UnboundedSender<usize>,
    count: AtomicUsize,
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

struct FailedSave {
    count: AtomicUsize,
    inner: Arc<JournalHistory>,
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
async fn review_memory_default_precheck_cannot_hold_connection_acceptance() {
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let caller = connection.caller();
    let resources: Arc<dyn ResourceResolver> = Arc::new(Resources {
        model: model(Vec::new()),
        tools: ToolSet::default(),
    });
    let agent = crate::agent::AgentOwner::new(
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        "acceptance fixture".into(),
        AgentConfig::default(),
        Arc::clone(&access),
        resources,
    );
    let create = |make_default| {
        agent.create_session_async_owned(
            OperationId::new().unwrap(),
            SessionOptions::default(),
            None,
            make_default,
        )
    };
    let first = create(true).await.unwrap().session_id.unwrap();
    let child_id = create(false).await.unwrap().session_id.unwrap();
    let child = agent.session(&child_id).unwrap();
    let baseline = Arc::strong_count(&child);
    let held = child.state.lock().unwrap();
    let setter_agent = Arc::clone(&agent);
    let setter_caller = caller.clone();
    let child_id = child.id.clone();
    let setter =
        std::thread::spawn(move || setter_agent.set_default(&setter_caller, Some(child_id)));
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while Arc::strong_count(&child) == baseline && std::time::Instant::now() < limit {
        std::thread::yield_now();
    }
    let reached_child = Arc::strong_count(&child) > baseline;
    // This holds only the child state as a test fixture. The real set_default
    // operation must not own the connection-acceptance mutex while waiting here.
    let (revoked, observed) = std::sync::mpsc::channel();
    let closer = std::thread::spawn(move || {
        connection.close();
        let _ = revoked.send(());
    });
    let revoked_during_precheck = observed
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok();
    drop(held);
    closer.join().unwrap();
    let result = setter.join().unwrap();
    assert!(
        reached_child,
        "setter must retain the actual target while checking its state"
    );
    assert!(
        revoked_during_precheck,
        "child precheck must not block connection revocation"
    );
    assert_eq!(
        result.unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    let replacement = access.connect().unwrap();
    assert_eq!(
        agent
            .info(&replacement.caller())
            .unwrap()
            .default_session_id,
        Some(first)
    );
}

// This fixture injects write interruptions into the real production backend.
// It does not mock storage acknowledgement, Runtime, or the Session owner.
struct FailureJournal {
    inner: Arc<JournalHistory>,
    failures: Mutex<VecDeque<Option<crate::backend::TestAppendFault>>>,
    appends: AtomicUsize,
    reconciles: AtomicUsize,
    reconcile_gate: Option<Arc<Semaphore>>,
    reconcile_entered: mpsc::UnboundedSender<()>,
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
fn failure_setup(
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
async fn blocked(fixture: &Fixture) -> TurnInfo {
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
fn assert_single_terminal(fixture: &Fixture, outcome: TurnOutcome, partials: usize) {
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
        let before = std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap();
        assert_eq!(backend.appends.load(Ordering::SeqCst), 1);
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        until(&fixture, |state| state.completed.len() == 1).await;
        assert_single_terminal(&fixture, TurnOutcome::Failed, 0);
        let after = std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap();
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
            std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap(),
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
            let bytes = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&path, bytes.replace("original input", "modified input")).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
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
            assert_eq!(std::fs::read(&path).unwrap(), before);
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

struct PartialUntilCancelled {
    descriptor: minicore_runtime::model::ModelDescriptor,
    consumed_delta: Arc<Semaphore>,
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

pub(super) async fn original_save_fixture() -> Fixture {
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

#[test]
fn idle_closed_memory_reclaims_bodies_and_root_without_a_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(dir.path().join("root")).unwrap();
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let session = SessionController::new(
        SessionId::new().unwrap(),
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        SessionConfig::default(),
        access,
        Arc::new(Resources {
            model: model(Vec::new()),
            tools: ToolSet::default(),
        }),
        Arc::new(MemoryHistory),
        vec![HistoryItem::Summary(
            minicore_runtime::history::SummaryHistory {
                content: Text::new("provided memory fixture context"),
            },
        )],
        Arc::new(AtomicBool::new(true)),
    )
    .hold_root(Some(Arc::new(root)));
    let observer = session.subscribe_state(&connection.caller()).unwrap();
    let retained = session.clone();
    assert_eq!(session.history(&connection.caller()).unwrap().len(), 1);
    assert!(DataRoot::acquire(dir.path().join("root")).is_err());
    let (_, records) = session
        .begin_close_owned(OperationId::new().unwrap())
        .unwrap();
    diagnostic::emit(records);
    assert!(session.retire_closed_immediately_owned().unwrap());
    assert_eq!(
        observer.snapshot().unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    assert_eq!(
        retained.history(&connection.caller()).unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    assert!(retained.state.lock().unwrap().history.is_empty());
    let reopened = DataRoot::acquire(dir.path().join("root")).unwrap();
    assert!(
        !dir.path().join("root/agents").exists(),
        "memory creates no Session JSONL"
    );
    drop(reopened);
}

struct ConstructorPanicHistory;
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
async fn save_acknowledgement_does_not_replace_actual_worker_retirement() {
    let dir = tempfile::tempdir().unwrap();
    let root_path = dir.path().join("root");
    let root = DataRoot::acquire(&root_path).unwrap();
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let session = SessionController::new(
        SessionId::new().unwrap(),
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        SessionConfig::default(),
        access,
        Arc::new(Resources {
            model: model(Vec::new()),
            tools: ToolSet::default(),
        }),
        Arc::new(MemoryHistory),
        Vec::new(),
        Arc::new(AtomicBool::new(true)),
    )
    .hold_root(Some(Arc::new(root)));
    let gate = Arc::new(Semaphore::new(0));
    let worker_gate = gate.clone();
    // Explicit worker-retirement fixture, not a fabricated Runtime execution:
    // all public save/business flags are settled while the actual task is alive.
    session.state.lock().unwrap().driver_task = Some(tokio::spawn(async move {
        worker_gate.acquire().await.unwrap().forget();
    }));
    session.begin_host_shutdown_owned(true);
    assert!(session.shutdown_status_owned().quiescent);
    let owner = session.clone();
    let release = tokio::spawn(async move { owner.release_host_resources_owned().await });
    until_releasing(&session).await;
    assert!(!release.is_finished());
    assert!(DataRoot::acquire(&root_path).is_err());
    assert_eq!(
        session.pause(&connection.caller()).unwrap_err().kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
    gate.add_permits(1);
    release.await.unwrap().unwrap();
    assert!(DataRoot::acquire(&root_path).is_ok());
}
async fn until_releasing(session: &Arc<SessionController>) {
    // The state claim is the deterministic gate; yields do not guess elapsed I/O time.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !session.state.lock().unwrap().releasing {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

mod history_read;

mod resource_resume;

/// Holds the first real journal save at a known Runtime/Session handoff, and
/// optionally makes that original append fail without discarding its range.
struct TerminalClaimJournal {
    inner: Arc<JournalHistory>,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    first: AtomicBool,
    fail_first: bool,
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
fn terminal_claim_fixture(
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

struct CancelDropPanicModel {
    descriptor: ModelDescriptor,
    entered: Arc<Semaphore>,
}
struct CancelDropPanicStream(Option<Arc<Semaphore>>);
impl futures_util::Stream for CancelDropPanicStream {
    type Item = Result<ModelEvent, minicore_runtime::model::ModelError>;
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

mod compaction;

mod deletion;

mod task071_discovery;
