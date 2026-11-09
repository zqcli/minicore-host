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
    model: Arc<FixtureModel>,
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
struct Fixture {
    _dir: tempfile::TempDir,
    store: SessionStore,
    access: Arc<ConnectionRegistry>,
    connection: crate::ConnectionHandle,
    session: Arc<SessionController>,
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
        ModelEvent::text_delta(text).unwrap(),
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
        ModelEvent::tool_call_arguments_delta("tool-original-id".parse().unwrap(), "{}").unwrap(),
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
    let model = model(vec![vec![
        ModelEvent::text_delta("unfinished actual text").unwrap(),
    ]]);
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
    let model = model(vec![vec![
        ModelEvent::text_delta("partial before primary stream failure").unwrap(),
    ]]);
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
        FaultKind::TurnModelStreamInterrupted
    );
    assert_eq!(
        active.info.persistence_fault.as_ref().unwrap().kind(),
        FaultKind::SessionHistoryWriteFailed
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
            .cancel(&fixture.connection.caller(), None, false)
            .unwrap()
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
