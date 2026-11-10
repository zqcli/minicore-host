//! Real Runtime, Session owners and temporary journals. The model below is an
//! explicitly scripted fixture; only the separate HTTP tests use a provider.
use super::*;
use crate::backend::TestAppendFault;
use minicore_runtime::{
    AssistantHistory, UserHistory, UserMessageKind,
    model::{AssistantPart, ModelMessage, Usage},
    prompt::{PromptFuture, PromptProvider, PromptRequest},
};
use std::time::Duration;

struct UtilityModel {
    descriptor: ModelDescriptor,
    utility_calls: AtomicUsize,
    requests: Mutex<Vec<(bool, ModelRequest, ModelCallContext)>>,
    scripts: Mutex<VecDeque<Vec<ModelEvent>>>,
    utility_entered: Semaphore,
    utility_gate: Option<Arc<Semaphore>>,
}
impl UtilityModel {
    fn new(scripts: Vec<Vec<ModelEvent>>, gated: bool) -> Arc<Self> {
        Arc::new(Self {
            descriptor: ModelDescriptor::new(
                "test/model".parse().unwrap(),
                4096,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
            utility_calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            scripts: Mutex::new(scripts.into()),
            utility_entered: Semaphore::new(0),
            utility_gate: gated.then(|| Arc::new(Semaphore::new(0))),
        })
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.utility_entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
}
impl Model for UtilityModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        let utility = matches!(request.messages().first(),Some(ModelMessage::System(text)) if text.starts_with("Summarize historical conversation data"));
        self.requests
            .lock()
            .unwrap()
            .push((utility, request, context));
        if utility {
            self.utility_calls.fetch_add(1, Ordering::SeqCst);
        }
        Box::pin(async move {
            let events = if utility {
                self.utility_entered.add_permits(1);
                if let Some(gate) = &self.utility_gate {
                    gate.acquire().await.unwrap().forget();
                }
                final_events("Summary: retain the original task and recorded outcomes.")
            } else {
                self.scripts
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("declared normal model response")
            };
            Ok(Box::pin(futures_util::stream::iter(events.into_iter().map(Ok))) as ModelStream)
        })
    }
}
struct HistoryPrompt;
impl PromptProvider for HistoryPrompt {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        Box::pin(crate::prepare_prompt_history(request))
    }
}
struct UtilityResources {
    model: Arc<dyn Model>,
    tools: ToolSet,
    prepare_entered: Semaphore,
    prepare_gate: Option<Arc<Semaphore>>,
}
impl UtilityResources {
    fn new(model: Arc<dyn Model>, tools: ToolSet) -> Arc<Self> {
        Arc::new(Self {
            model,
            tools,
            prepare_entered: Semaphore::new(0),
            prepare_gate: None,
        })
    }
}
impl ResourceResolver for UtilityResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    self.tools.clone(),
                    None,
                    Arc::new(HistoryPrompt),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        _: crate::PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.prepare_entered.add_permits(1);
            if let Some(gate) = &self.prepare_gate {
                gate.acquire().await.unwrap().forget();
            }
            self.resolve(config).await
        })
    }
}
fn seed_entries(pairs: usize) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    for index in 0..pairs {
        let turn = TurnId::new().unwrap();
        let loop_id = LoopId::new().unwrap();
        entries.push(HistoryEntry::Input {
            turn_id: turn.clone(),
            message_id: MessageId::new().unwrap(),
            source: MessageSource::User,
            mode: MessageMode::FollowUp,
            in_reply_to: None,
            user: UserHistory {
                loop_id,
                kind: UserMessageKind::Prompt,
                input: UserInput::text(format!(
                    "old task {index} {}",
                    "history detail ".repeat(95)
                ))
                .unwrap(),
            },
        });
        entries.push(HistoryEntry::Runtime {
            turn_id: turn.clone(),
            response_index: 0,
            item: HistoryItem::Assistant(AssistantHistory {
                loop_id,
                request_index: 0,
                model: "test/model".parse().unwrap(),
                reasoning: ReasoningPreference::Auto,
                content: vec![AssistantPart::Text(
                    format!("old answer {index} {}", "recorded fact ".repeat(100)).into(),
                )],
                provider_replay: None,
                finish_reason: ModelFinishReason::Stop,
                usage: Usage::default(),
            }),
        });
        entries.push(HistoryEntry::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
            reason: None,
        });
    }
    entries
}
async fn seed(fixture: &Fixture, automatic: bool) {
    fixture
        .session
        .backend
        .append(seed_entries(3))
        .await
        .unwrap();
    {
        let mut state = fixture.session.state.lock().unwrap();
        state.info.config.execution.compaction = CompactionSettings {
            enabled: automatic,
            trigger_percent: 50,
            target_percent: 20,
        };
    }
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
    until(fixture, |state| state.history_loaded && !state.preparing).await;
}
fn start(fixture: &Fixture) -> OperationId {
    let id = OperationId::new().unwrap();
    let receipt = fixture
        .session
        .compact(
            &fixture.connection.caller(),
            CompactRequest {
                operation_id: id.clone(),
            },
        )
        .unwrap();
    assert!(matches!(
        receipt.result.state,
        CompactionState::Preparing { .. }
    ));
    id
}
async fn compacted(fixture: &Fixture) -> CompactionActivity {
    until(fixture, |state| {
        state.compaction.is_none() && state.last_compaction.is_some()
    })
    .await;
    fixture
        .session
        .state
        .lock()
        .unwrap()
        .last_compaction
        .clone()
        .unwrap()
}
fn markers(fixture: &Fixture) -> usize {
    records(fixture)
        .iter()
        .filter(|record| matches!(record.entry, HistoryEntry::Compaction { .. }))
        .count()
}
fn cancel(fixture: &Fixture, target: OperationId) -> CompactCancelReceipt {
    fixture
        .session
        .compact_cancel(
            &fixture.connection.caller(),
            CompactCancelRequest {
                operation_id: OperationId::new().unwrap(),
                target_operation_id: target,
            },
        )
        .unwrap()
}

#[tokio::test]
async fn compaction_repeated_durable_coverage_reloads_without_touching_raw_history() {
    let model = UtilityModel::new(vec![final_events(&"new answer ".repeat(200))], false);
    let fixture = setup(
        UtilityResources::new(model.clone(), ToolSet::default()),
        |inner| inner,
    );
    seed(&fixture, false).await;
    let path = fixture.store.canonical_path().join("history.jsonl");
    let original = std::fs::read(&path).unwrap();
    let raw = fixture
        .session
        .history(&fixture.connection.caller())
        .unwrap();
    start(&fixture);
    let first = compacted(&fixture).await;
    assert!(matches!(
        first.state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted {
                persistence: PersistenceState::Persisted,
                ..
            }
        }
    ));
    assert!(std::fs::read(&path).unwrap().starts_with(&original));
    assert_eq!(
        fixture
            .session
            .history(&fixture.connection.caller())
            .unwrap(),
        raw
    );
    let coverage = fixture
        .session
        .state
        .lock()
        .unwrap()
        .projection
        .as_ref()
        .unwrap()
        .covered_items;
    send(&fixture, &"new task ".repeat(250));
    until(&fixture, |state| {
        state.completed.len() == 1 && !state.driver_running
    })
    .await;
    let raw = fixture
        .session
        .history(&fixture.connection.caller())
        .unwrap();
    start(&fixture);
    compacted(&fixture).await;
    assert_eq!(markers(&fixture), 2);
    let expected = {
        let state = fixture.session.state.lock().unwrap();
        state
            .projection
            .as_ref()
            .unwrap()
            .project(&state.history)
            .unwrap()
    };
    assert!(
        fixture
            .session
            .state
            .lock()
            .unwrap()
            .projection
            .as_ref()
            .unwrap()
            .covered_items
            > coverage
    );
    fixture
        .session
        .control(
            &fixture.connection.caller(),
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            SessionControlKind::Unload,
        )
        .unwrap();
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
    until(&fixture, |state| state.history_loaded && !state.preparing).await;
    let state = fixture.session.state.lock().unwrap();
    assert_eq!(state.history, raw);
    assert_eq!(
        state
            .projection
            .as_ref()
            .unwrap()
            .project(&state.history)
            .unwrap(),
        expected
    );
    assert_eq!(
        expected
            .iter()
            .filter(|item| matches!(item, HistoryItem::Summary(_)))
            .count(),
        1
    );
    assert_eq!(state.history_position, records(&fixture).len() as u64);
}

#[tokio::test]
async fn compaction_manual_keeps_both_queues_and_explicit_pause_and_stale_cancel_target() {
    let model = UtilityModel::new(Vec::new(), true);
    let fixture = setup(
        UtilityResources::new(model.clone(), ToolSet::default()),
        |inner| inner,
    );
    seed(&fixture, false).await;
    fixture.session.pause(&fixture.connection.caller()).unwrap();
    let first = start(&fixture);
    let formed_first = fixture.session.control_receipts_owned(&first);
    model.entered().await;
    assert_eq!(fixture.session.control_receipts_owned(&first), formed_first);
    send(&fixture, "follow-up queued");
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::Steer,
                text: "steer queued".into(),
                in_reply_to: None,
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .session
            .control(
                &fixture.connection.caller(),
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap()
                },
                SessionControlKind::Unload
            )
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionBusy)
    );
    let claimed = cancel(&fixture, first.clone());
    assert!(claimed.result.cancel_claimed);
    assert!(!cancel(&fixture, first.clone()).result.cancel_claimed);
    let result = compacted(&fixture).await;
    assert!(matches!(
        result.state,
        CompactionState::Finished {
            result: CompactionOutcome::Cancelled
        }
    ));
    assert_eq!(markers(&fixture), 0);
    let second = start(&fixture);
    let formed_second = fixture.session.control_receipts_owned(&second);
    model.entered().await;
    let late = cancel(&fixture, first.clone());
    assert!(!late.result.cancel_claimed);
    assert_eq!(late.result.target.state, result.state);
    assert!(
        !fixture
            .session
            .state
            .lock()
            .unwrap()
            .compaction
            .as_ref()
            .unwrap()
            .activity
            .cancellation_requested
    );
    model.utility_gate.as_ref().unwrap().add_permits(10);
    compacted(&fixture).await;
    // Later work replaces the public latest activity, not the original
    // operation's actual result or any already formed control receipt.
    let late = cancel(&fixture, first.clone());
    assert!(!late.result.cancel_claimed);
    assert_eq!(late.result.target.state, result.state);
    assert!(late.result.target.cancellation_requested);
    assert_eq!(fixture.session.control_receipts_owned(&first), formed_first);
    assert_eq!(
        fixture.session.control_receipts_owned(&second),
        formed_second
    );
    assert_eq!(
        fixture
            .session
            .control_receipts_owned(&claimed.operation_id),
        vec![Receipt::CompactCancel(claimed)]
    );
    let state = fixture.session.state.lock().unwrap();
    assert!(state.paused);
    assert!(state.active.is_none());
    assert!(state.completed.is_empty());
    assert_eq!(state.next_step.len(), 1);
    assert_eq!(state.next_turn.len(), 1);
    assert!(state.last_compaction.as_ref().unwrap().state.finished());
}

#[tokio::test]
async fn compaction_reused_operation_id_does_not_redirect_cancellation_to_later_work() {
    let model = UtilityModel::new(Vec::new(), true);
    let fixture = setup(
        UtilityResources::new(model.clone(), ToolSet::default()),
        |inner| inner,
    );
    seed(&fixture, false).await;
    let operation = start(&fixture);
    let formed = fixture.session.control_receipts_owned(&operation);
    model.entered().await;
    assert!(cancel(&fixture, operation.clone()).result.cancel_claimed);
    compacted(&fixture).await;
    let second = fixture
        .session
        .compact(
            &fixture.connection.caller(),
            CompactRequest {
                operation_id: operation.clone(),
            },
        )
        .unwrap();
    model.entered().await;
    let error = fixture
        .session
        .compact_cancel(
            &fixture.connection.caller(),
            CompactCancelRequest {
                operation_id: OperationId::new().unwrap(),
                target_operation_id: operation.clone(),
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), Some(FaultKind::HostReceiptAmbiguous));
    assert!(
        !fixture
            .session
            .state
            .lock()
            .unwrap()
            .compaction
            .as_ref()
            .unwrap()
            .activity
            .cancellation_requested
    );
    model.utility_gate.as_ref().unwrap().add_permits(10);
    let completed = compacted(&fixture).await;
    assert!(matches!(
        completed.state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted { .. }
        }
    ));
    assert_eq!(
        fixture.session.control_receipts_owned(&operation),
        [formed, vec![Receipt::Compact(second)]].concat()
    );
}

#[tokio::test]
async fn compaction_manual_and_turn_preparation_share_one_admission_boundary() {
    let model = UtilityModel::new(vec![final_events("done")], false);
    let gate = Arc::new(Semaphore::new(0));
    let resources = Arc::new(UtilityResources {
        model,
        tools: ToolSet::default(),
        prepare_entered: Semaphore::new(0),
        prepare_gate: Some(gate.clone()),
    });
    let fixture = setup(resources.clone(), |inner| inner);
    seed(&fixture, false).await;
    send(&fixture, "prepare this turn");
    resources.prepare_entered.acquire().await.unwrap().forget();
    assert_eq!(
        fixture
            .session
            .compact(
                &fixture.connection.caller(),
                CompactRequest {
                    operation_id: OperationId::new().unwrap()
                }
            )
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionBusy)
    );
    assert!(fixture.session.state.lock().unwrap().next_turn.len() == 1);
    gate.add_permits(1);
    until(&fixture, |state| state.completed.len() == 1).await;
}

/// Gate only the real compaction marker, leaving ordinary Runtime batches alone.
struct CompactionJournal {
    inner: Arc<JournalHistory>,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    failure: Mutex<Option<TestAppendFault>>,
    appends: AtomicUsize,
    reconciles: AtomicUsize,
}
impl HistoryBackend for CompactionJournal {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.inner.load()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        Box::pin(async move {
            if entries
                .iter()
                .any(|entry| matches!(entry, HistoryEntry::Compaction { .. }))
            {
                self.appends.fetch_add(1, Ordering::SeqCst);
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
                if let Some(failure) = self.failure.lock().unwrap().take() {
                    self.inner.inject_next_append_fault(failure);
                }
            }
            self.inner.append(entries).await
        })
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        self.reconciles.fetch_add(1, Ordering::SeqCst);
        self.inner.reconcile()
    }
    fn release(&self) -> Pin<Box<dyn Future<Output = Result<(), Fault>> + Send + '_>> {
        self.inner.release()
    }
}
fn gated_fixture(
    model: Arc<UtilityModel>,
    fault: Option<TestAppendFault>,
) -> (Fixture, Arc<CompactionJournal>) {
    let mut backend = None;
    let fixture = setup(UtilityResources::new(model, ToolSet::default()), |inner| {
        let value = Arc::new(CompactionJournal {
            inner,
            entered: Arc::new(Semaphore::new(0)),
            release: Arc::new(Semaphore::new(0)),
            failure: Mutex::new(fault),
            appends: AtomicUsize::new(0),
            reconciles: AtomicUsize::new(0),
        });
        backend = Some(value.clone());
        value
    });
    (fixture, backend.unwrap())
}
#[tokio::test]
async fn compaction_commit_claim_defeats_cancel_and_force_waits_for_original_save() {
    let model = UtilityModel::new(Vec::new(), false);
    let (fixture, backend) = gated_fixture(model, None);
    seed(&fixture, false).await;
    let id = start(&fixture);
    backend.entered.acquire().await.unwrap().forget();
    assert!(!cancel(&fixture, id).result.cancel_claimed);
    fixture.session.begin_host_shutdown_owned(true);
    assert!(!fixture.session.shutdown_status_owned().quiescent);
    assert!(
        fixture
            .session
            .release_host_resources_owned()
            .await
            .is_err()
    );
    assert!(fixture.session.state.lock().unwrap().projection.is_none());
    backend.release.add_permits(1);
    compacted(&fixture).await;
    assert_eq!(markers(&fixture), 1);
    fixture
        .session
        .release_host_resources_owned()
        .await
        .unwrap();
}

#[tokio::test]
async fn compaction_save_reconcile_keeps_original_candidate_and_never_calls_utility_twice() {
    for fault in [TestAppendFault::NoWrite, TestAppendFault::FullWrite] {
        let model = UtilityModel::new(Vec::new(), false);
        let (fixture, backend) = gated_fixture(model.clone(), Some(fault));
        seed(&fixture, false).await;
        start(&fixture);
        backend.entered.acquire().await.unwrap().forget();
        backend.release.add_permits(1);
        until(&fixture, |state| {
            state
                .history_work
                .as_ref()
                .is_some_and(|work| work.fault.is_some())
        })
        .await;
        let utility = model.utility_calls.load(Ordering::SeqCst);
        assert!(fixture.session.state.lock().unwrap().projection.is_none());
        assert_eq!(
            fixture
                .session
                .control(
                    &fixture.connection.caller(),
                    SessionControlRequest {
                        operation_id: OperationId::new().unwrap()
                    },
                    SessionControlKind::Unload
                )
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionBusy)
        );
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        compacted(&fixture).await;
        assert_eq!(backend.appends.load(Ordering::SeqCst), 1);
        assert_eq!(backend.reconciles.load(Ordering::SeqCst), 1);
        assert_eq!(model.utility_calls.load(Ordering::SeqCst), utility);
        assert_eq!(markers(&fixture), 1);
        assert!(fixture.session.state.lock().unwrap().block_reason.is_none());
    }
}

// Reuse the existing real append gate, adding only its original-save reconcile
// delegation. There is no simulated success or owner-result injection here.
struct CompactionEndJournal(Arc<GatedJournal>);
impl HistoryBackend for CompactionEndJournal {
    fn mode(&self) -> StorageMode {
        self.0.mode()
    }
    fn load(&self) -> LoadFuture<'_> {
        self.0.load()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        self.0.append(entries)
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        self.0.inner.reconcile()
    }
}

#[tokio::test]
async fn compaction_original_failure_survives_cancel_and_independent_save_failure_once() {
    for (cancel_first, fail_save) in [(false, true), (true, false), (true, true)] {
        // Explicit scripted model fixture returns an empty utility stream.
        // The production utility engine rejects that actual response.
        let model = Arc::new(FixtureModel {
            descriptor: ModelDescriptor::new(
                "test/model".parse().unwrap(),
                4096,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
            calls: AtomicUsize::new(0),
            scripts: Mutex::new(VecDeque::from([Vec::new()])),
        });
        let (entered, mut received) = mpsc::unbounded_channel();
        let mut backend = None;
        let fixture = setup(
            UtilityResources::new(model.clone(), ToolSet::default()),
            |inner| {
                let journal = Arc::new(GatedJournal {
                    inner,
                    gates: Mutex::new(VecDeque::new()),
                    entered,
                    count: AtomicUsize::new(0),
                });
                backend = Some(journal.clone());
                Arc::new(CompactionEndJournal(journal))
            },
        );
        seed(&fixture, true).await;
        assert_eq!(received.recv().await, Some(0));
        let gate = Arc::new(Semaphore::new(0));
        backend
            .as_ref()
            .unwrap()
            .gates
            .lock()
            .unwrap()
            .push_back(gate.clone());
        send(&fixture, "original utility failure before cancellation");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), received.recv())
                .await
                .unwrap(),
            Some(1)
        );
        let (turn, original_fault, binding) = {
            let state = fixture.session.state.lock().unwrap();
            let active = state.active.as_ref().unwrap();
            assert_eq!(active.info.state, TurnState::Running);
            let CompactionState::Finished {
                result: CompactionOutcome::Failed { fault },
            } = &state.last_compaction.as_ref().unwrap().state
            else {
                panic!("actual utility must have failed")
            };
            assert_eq!(fault.kind(), Some(FaultKind::SessionCompactionFailed));
            let handoff = active
                .compaction_failure
                .as_ref()
                .expect("original bound failure");
            assert_eq!(&handoff.fault, fault);
            assert!(!handoff.history);
            (
                active.info.id.clone(),
                fault.clone(),
                handoff.binding.clone(),
            )
        };
        let path = fixture.store.canonical_path().join("history.jsonl");
        let original = path.with_extension("original");
        if fail_save {
            // Real production Journal identity refusal, not injected errno.
            std::fs::rename(&path, &original).unwrap();
            std::fs::write(&path, b"").unwrap();
        }
        let outcome = if cancel_first {
            let (cancel, _) = fixture
                .session
                .cancel(&fixture.connection.caller(), Some(&turn), true, None)
                .unwrap();
            assert_eq!(cancel.status, CancelStatus::Claimed);
            TurnOutcome::Cancelled
        } else {
            TurnOutcome::Failed
        };
        let primary = (!cancel_first).then(|| original_fault.clone());
        let cleanup = if cancel_first {
            vec![original_fault.clone()]
        } else {
            Vec::new()
        };
        gate.add_permits(1);
        if fail_save {
            let pending = blocked(&fixture).await;
            assert_eq!(pending.outcome, Some(outcome));
            assert_eq!(pending.fault, primary);
            assert_eq!(pending.cleanup_faults, cleanup);
            assert_eq!(
                pending.persistence_fault.as_ref().unwrap().kind(),
                Some(FaultKind::SessionHistoryWriteFailed)
            );
            {
                // Pure identity probes use the actual held owner/report, with
                // only one deliberately mismatched binding scalar per probe.
                let state = fixture.session.state.lock().unwrap();
                let active = state.active.as_ref().unwrap();
                let report = active.report.as_ref().unwrap();
                assert!(active.compaction_failure.is_none(), "handoff consumed once");
                let exact = crate::session::compaction::CompactionFailure {
                    binding: binding.clone(),
                    fault: original_fault.clone(),
                    history: false,
                };
                assert!(exact.matches(active, report));
                let mut mismatches = Vec::new();
                for index in 0..4 {
                    let mut wrong = binding.clone();
                    let crate::session::compaction::CompactionBinding::Automatic {
                        turn_id,
                        load_id,
                        loop_id,
                        request_index,
                    } = &mut wrong
                    else {
                        unreachable!()
                    };
                    match index {
                        0 => *turn_id = TurnId::new().unwrap(),
                        1 => *load_id = LoadId::new().unwrap(),
                        2 => *loop_id = LoopId::new().unwrap(),
                        _ => *request_index += 1,
                    }
                    mismatches.push(wrong);
                }
                mismatches.push(crate::session::compaction::CompactionBinding::Manual {
                    operation_id: OperationId::new().unwrap(),
                    receipt_index: 0,
                    config_revision: 0,
                });
                for binding in mismatches {
                    assert!(
                        !crate::session::compaction::CompactionFailure {
                            binding,
                            fault: original_fault.clone(),
                            history: false,
                        }
                        .matches(active, report)
                    );
                }
                let mut other_loop = (**report).clone();
                other_loop.loop_id = LoopId::new().unwrap();
                assert!(!exact.matches(active, &other_loop));
            }
            std::fs::remove_file(&path).unwrap();
            std::fs::rename(&original, &path).unwrap();
            fixture
                .session
                .resume(&fixture.connection.caller())
                .unwrap();
        }
        let result = fixture
            .session
            .wait_turn(&fixture.connection.caller(), &turn)
            .await
            .unwrap();
        assert_eq!(result.info.outcome, Some(outcome));
        assert_eq!(result.info.fault, primary);
        assert_eq!(result.info.cleanup_faults, cleanup);
        assert_eq!(result.info.persistence, PersistenceState::Persisted);
        assert!(result.info.persistence_fault.is_none());
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        assert_eq!(markers(&fixture), 0);
    }
}

#[tokio::test]
async fn compaction_prompt_cancellation_fences_terminal_handoff_then_resumes_original_range() {
    let model = UtilityModel::new(Vec::new(), false);
    let (fixture, backend) = gated_fixture(model.clone(), Some(TestAppendFault::NoWrite));
    seed(&fixture, true).await;
    let raw = fixture
        .session
        .history(&fixture.connection.caller())
        .unwrap();
    send(&fixture, "fresh input must survive cancellation");
    backend.entered.acquire().await.unwrap().forget();
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
    fixture
        .session
        .cancel(&fixture.connection.caller(), Some(&turn), true, None)
        .unwrap();
    // Wait for the real terminal callback to reach the Session save fence.
    // Utility child cancellation alone would not prove this: successful EOF
    // already cancels that per-call token.
    let history_waiting = fixture
        .session
        .state
        .lock()
        .unwrap()
        .compaction
        .as_ref()
        .unwrap()
        .history_waiting
        .clone();
    tokio::time::timeout(Duration::from_secs(5), history_waiting.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(
        fixture
            .session
            .history(&fixture.connection.caller())
            .unwrap(),
        raw
    );
    assert_eq!(
        fixture
            .session
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .unwrap()
            .accepted,
        0
    );
    backend.release.add_permits(1);
    until(&fixture, |state| {
        !state.driver_running
            && state
                .active
                .as_ref()
                .is_some_and(|active| active.info.state == TurnState::Finalizing)
    })
    .await;
    assert_eq!(
        fixture
            .session
            .history(&fixture.connection.caller())
            .unwrap(),
        raw
    );
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    let result = fixture
        .session
        .wait_turn(&fixture.connection.caller(), &turn)
        .await
        .unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert!(
        matches!(&result.history[..],[HistoryItem::User(user)] if user.input.as_text()=="fresh input must survive cancellation")
    );
    assert_eq!(markers(&fixture), 1);
    assert_eq!(backend.appends.load(Ordering::SeqCst), 1);
    assert!(
        model
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|(utility, _, _)| *utility)
    );
}

#[tokio::test]
async fn compaction_automatic_af_rebuild_uses_new_projection_and_preserves_queued_steer() {
    let model = UtilityModel::new(vec![final_events("normal response")], true);
    let fixture = setup(
        UtilityResources::new(model.clone(), ToolSet::default()),
        |inner| inner,
    );
    seed(&fixture, true).await;
    send(&fixture, "initial fresh input");
    model.entered().await;
    let before = fixture.session.state.lock().unwrap().history_position;
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::Steer,
                text: "A/F new instruction verbatim".into(),
                in_reply_to: None,
            },
        )
        .unwrap();
    assert_eq!(
        fixture.session.state.lock().unwrap().history_position,
        before
    );
    model.utility_gate.as_ref().unwrap().add_permits(10);
    until(&fixture, |state| state.completed.len() == 1).await;
    let requests = model.requests.lock().unwrap();
    let normal = requests
        .iter()
        .filter(|(utility, _, _)| !*utility)
        .collect::<Vec<_>>();
    assert_eq!(normal.len(), 1);
    assert_eq!(normal[0].2.request_index, 0);
    let json = serde_json::to_string(&normal[0].1).unwrap();
    assert!(json.contains("Conversation summary:"));
    assert!(json.contains("A/F new instruction verbatim"));
    assert!(json.contains("initial fresh input"));
    assert_eq!(markers(&fixture), 1);
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.values().next().unwrap();
    assert_eq!(result.info.requests, 1);
    assert_eq!(result.history.len(), 3);
    assert!(state.next_step.is_empty());
}

struct VerboseTool {
    spec: ToolSpec,
    calls: Arc<AtomicUsize>,
}
impl Tool for VerboseTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("actual tool output ".repeat(110)).unwrap(),
            ))
        })
    }
}
fn verbose_tool_response(index: usize) -> Vec<ModelEvent> {
    let id: minicore_runtime::ToolCallId = format!("count-{index}").parse().unwrap();
    vec![
        ModelEvent::text_delta("tool decision context ".repeat(100)),
        ModelEvent::ToolCallStart {
            tool_call_id: id.clone(),
            tool_name: "count".parse().unwrap(),
        },
        ModelEvent::tool_call_arguments_delta(id.clone(), "{}"),
        ModelEvent::ToolCallEnd { tool_call_id: id },
        ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        },
    ]
}
#[tokio::test]
async fn compaction_long_public_turn_covers_its_own_saved_tool_batches_without_reslicing_result() {
    let model = UtilityModel::new(
        vec![
            verbose_tool_response(0),
            verbose_tool_response(1),
            verbose_tool_response(2),
            final_events("finished one long turn"),
        ],
        false,
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolSet::builder();
    tools.register(VerboseTool {
        spec: ToolSpec::new(
            "count".parse().unwrap(),
            "count once",
            serde_json::json!({}),
        )
        .unwrap(),
        calls: calls.clone(),
    });
    let fixture = setup(
        UtilityResources::new(model.clone(), tools.build().unwrap()),
        |inner| inner,
    );
    {
        let mut state = fixture.session.state.lock().unwrap();
        state.info.config.tools = vec![ToolBinding {
            name: "count".into(),
        }];
        state.info.config.execution.compaction = CompactionSettings {
            enabled: true,
            trigger_percent: 45,
            target_percent: 20,
        };
    }
    send(&fixture, &"long original input ".repeat(80));
    until(&fixture, |state| state.completed.len() == 1).await;
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.values().next().unwrap();
    assert_eq!(
        result.info.outcome,
        Some(TurnOutcome::Completed),
        "{:?}",
        result.info
    );
    assert_eq!(result.info.requests, 4);
    assert_eq!(result.info.tool_rounds, 3);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(result.history.len(), 8);
    assert_eq!(state.history, result.history);
    assert!(
        state.projection.as_ref().unwrap().covered_items >= 3,
        "first same-Turn tool batch must be covered"
    );
    let responses = result
        .history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::Assistant(value) => Some(value.request_index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(responses, vec![0, 1, 2, 3]);
    assert!(model.utility_calls.load(Ordering::SeqCst) > 0);
    assert_eq!(
        model
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(utility, _, _)| !*utility)
            .count(),
        4
    );
}

#[tokio::test]
async fn compaction_partial_marker_failure_stays_blocked_and_force_does_not_release_owner() {
    let model = UtilityModel::new(Vec::new(), false);
    let (fixture, backend) = gated_fixture(model.clone(), Some(TestAppendFault::Prefix(17)));
    seed(&fixture, false).await;
    start(&fixture);
    backend.entered.acquire().await.unwrap().forget();
    backend.release.add_permits(1);
    until(&fixture, |state| {
        state
            .history_work
            .as_ref()
            .is_some_and(|work| work.fault.is_some())
    })
    .await;
    let before = std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap();
    fixture.session.begin_host_shutdown_owned(true);
    fixture
        .session
        .resume(&fixture.connection.caller())
        .unwrap();
    until(&fixture, |state| {
        state
            .history_work
            .as_ref()
            .is_some_and(|work| work.fault.is_some())
            && !state.saving
    })
    .await;
    assert_eq!(
        std::fs::read(fixture.store.canonical_path().join("history.jsonl")).unwrap(),
        before
    );
    assert!(
        fixture
            .session
            .release_host_resources_owned()
            .await
            .is_err()
    );
    assert!(fixture.session.state.lock().unwrap().projection.is_none());
    assert_eq!(backend.appends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn compaction_memory_keeps_projection_through_unload_without_creating_files_or_turn() {
    let directory = tempfile::tempdir().unwrap();
    let model = UtilityModel::new(Vec::new(), false);
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let mut config = SessionConfig {
        model: "test/model".parse().unwrap(),
        ..SessionConfig::default()
    };
    config.execution.compaction.target_percent = 20;
    let raw = seed_entries(3)
        .iter()
        .filter_map(HistoryEntry::runtime_item)
        .collect::<Vec<_>>();
    let session = SessionController::new(
        SessionId::new().unwrap(),
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        config,
        access,
        UtilityResources::new(model, ToolSet::default()),
        Arc::new(MemoryHistory),
        raw.clone(),
        Arc::new(AtomicBool::new(true)),
    );
    session.pause(&connection.caller()).unwrap();
    session
        .compact(
            &connection.caller(),
            CompactRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    let mut states = session.snapshots.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while session.state.lock().unwrap().compaction.is_some() {
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let projection = session.state.lock().unwrap().projection.clone().unwrap();
    session
        .control(
            &connection.caller(),
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            SessionControlKind::Unload,
        )
        .unwrap();
    let state = session.state.lock().unwrap();
    assert_eq!(state.projection.as_ref(), Some(&projection));
    assert_eq!(state.history, raw);
    assert!(state.paused);
    assert!(state.completed.is_empty());
    assert!(matches!(
        &state.last_compaction.as_ref().unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted {
                persistence: PersistenceState::NotApplicable,
                ..
            }
        }
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn compaction_old_missing_result_remains_unknown_and_no_tool_is_replayed() {
    let model = UtilityModel::new(vec![final_events("new response")], false);
    let fixture = setup(
        UtilityResources::new(model.clone(), ToolSet::default()),
        |inner| inner,
    );
    let mut entries = seed_entries(3);
    if let HistoryEntry::Runtime {
        item: HistoryItem::Assistant(assistant),
        ..
    } = &mut entries[1]
    {
        assistant.content.push(AssistantPart::ToolCall(
            minicore_runtime::model::ToolCall::new(
                "old-missing".parse().unwrap(),
                "historical_tool".parse().unwrap(),
                serde_json::json!({"path":"original"}),
                0,
            )
            .unwrap(),
        ));
        assistant.finish_reason = ModelFinishReason::ToolCalls;
    }
    fixture
        .session
        .backend
        .append(entries.clone())
        .await
        .unwrap();
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
    until(&fixture, |state| state.history_loaded && !state.preparing).await;
    {
        fixture
            .session
            .state
            .lock()
            .unwrap()
            .info
            .config
            .execution
            .compaction
            .target_percent = 20;
    }
    let raw = fixture
        .session
        .history(&fixture.connection.caller())
        .unwrap();
    start(&fixture);
    compacted(&fixture).await;
    assert_eq!(
        fixture
            .session
            .history(&fixture.connection.caller())
            .unwrap(),
        raw
    );
    let source = {
        let requests = model.requests.lock().unwrap();
        serde_json::to_string(&requests.iter().find(|(utility, _, _)| *utility).unwrap().1).unwrap()
    };
    assert!(source.contains("old-missing"));
    assert!(source.contains("missing; execution and outcome are unknown"));
    send(&fixture, "continue with recorded facts");
    until(&fixture, |state| state.completed.len() == 1).await;
    let state = fixture.session.state.lock().unwrap();
    assert!(
        state
            .history
            .iter()
            .all(|item| !matches!(item, HistoryItem::ToolResult(_)))
    );
    assert_eq!(state.completed.values().next().unwrap().info.tool_rounds, 0);
}

#[tokio::test]
async fn compaction_manual_first_holds_startup_while_other_session_runs_and_graceful_drains() {
    let model = UtilityModel::new(vec![final_events("queued turn after utility")], true);
    let resources = UtilityResources::new(model.clone(), ToolSet::default());
    let fixture = setup(resources.clone(), |inner| inner);
    seed(&fixture, false).await;
    start(&fixture);
    model.entered().await;
    send(&fixture, "unpaused queue waits for compaction");
    assert_eq!(resources.prepare_entered.available_permits(), 0);
    assert!(fixture.session.state.lock().unwrap().active.is_none());
    let other = setup(
        UtilityResources::new(
            UtilityModel::new(vec![final_events("other session runs")], false),
            ToolSet::default(),
        ),
        |inner| inner,
    );
    send(&other, "independent work");
    until(&other, |state| state.completed.len() == 1).await;
    fixture.session.begin_host_shutdown_owned(false);
    assert!(!fixture.session.shutdown_status_owned().drained);
    assert!(
        !fixture
            .session
            .state
            .lock()
            .unwrap()
            .compaction
            .as_ref()
            .unwrap()
            .activity
            .cancellation_requested
    );
    model.utility_gate.as_ref().unwrap().add_permits(10);
    until(&fixture, |state| {
        state.completed.len() == 1 && !state.driver_running
    })
    .await;
    assert!(fixture.session.shutdown_status_owned().drained);
    fixture
        .session
        .release_host_resources_owned()
        .await
        .unwrap();
}

#[tokio::test]
async fn compaction_close_cancels_utility_without_marker_and_retains_preclose_history() {
    let model = UtilityModel::new(Vec::new(), true);
    let fixture = setup(
        UtilityResources::new(model.clone(), ToolSet::default()),
        |inner| inner,
    );
    seed(&fixture, false).await;
    let raw = fixture
        .session
        .history(&fixture.connection.caller())
        .unwrap();
    start(&fixture);
    model.entered().await;
    fixture
        .session
        .close_owned(OperationId::new().unwrap())
        .unwrap();
    assert!(
        fixture
            .session
            .state
            .lock()
            .unwrap()
            .compaction
            .as_ref()
            .unwrap()
            .activity
            .cancellation_requested
    );
    let result = compacted(&fixture).await;
    assert!(matches!(
        result.state,
        CompactionState::Finished {
            result: CompactionOutcome::Cancelled
        }
    ));
    assert_eq!(markers(&fixture), 0);
    assert_eq!(
        fixture
            .session
            .history(&fixture.connection.caller())
            .unwrap(),
        raw
    );
    assert!(
        model
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|(_, _, context)| context.cancellation.is_cancelled())
    );
    assert_eq!(
        fixture.session.state.lock().unwrap().lifecycle,
        SessionLifecycle::Closing
    );
}

#[tokio::test]
async fn compaction_automatic_old_missing_tail_keeps_unknown_exchange_usable() {
    let model = UtilityModel::new(vec![final_events("new response")], false);
    let fixture = setup(
        UtilityResources::new(model.clone(), ToolSet::default()),
        |inner| inner,
    );
    let mut entries = seed_entries(3);
    if let HistoryEntry::Runtime {
        item: HistoryItem::Assistant(assistant),
        ..
    } = &mut entries[7]
    {
        assistant.content.push(AssistantPart::ToolCall(
            minicore_runtime::model::ToolCall::new(
                "old-tail-missing".parse().unwrap(),
                "historical_tool".parse().unwrap(),
                serde_json::json!({}),
                0,
            )
            .unwrap(),
        ));
        assistant.finish_reason = ModelFinishReason::ToolCalls;
    }
    fixture.session.backend.append(entries).await.unwrap();
    {
        let mut state = fixture.session.state.lock().unwrap();
        state.info.config.execution.compaction = CompactionSettings {
            enabled: true,
            trigger_percent: 50,
            target_percent: 10,
        };
    }
    send(&fixture, "new input after the historical missing result");
    until(&fixture, |state| state.completed.len() == 1).await;
    let state = fixture.session.state.lock().unwrap();
    let result = state.completed.values().next().unwrap();
    assert_eq!(
        result.info.outcome,
        Some(TurnOutcome::Completed),
        "{:?}; {:?}",
        result.info,
        state.last_compaction
    );
    assert_eq!(result.info.tool_rounds, 0);
    assert!(
        state
            .history
            .iter()
            .all(|item| !matches!(item, HistoryItem::ToolResult(_)))
    );
    let requests = model.requests.lock().unwrap();
    let normal = requests.iter().find(|(utility, _, _)| !*utility).unwrap();
    assert!(
        serde_json::to_string(&normal.1)
            .unwrap()
            .contains("current history has no corresponding result record")
    );
}
