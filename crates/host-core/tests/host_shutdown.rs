//! Acceptance checks: production Host/Journal/Runtime with a
//! deterministic in-process model fixture; no live provider or TUI coverage.
use futures_util::{StreamExt, stream};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use minicore_host_storage::{HistoryEntry, HistoryRecord};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

struct ModelFixture {
    descriptor: ModelDescriptor,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    calls: Arc<AtomicUsize>,
    partial: bool,
}
impl Model for ModelFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let partial = self.partial;
        let entered = self.entered.clone();
        if !partial {
            entered.add_permits(1);
        }
        let release = self.release.clone();
        Box::pin(async move {
            let tail = stream::once(async move {
                if partial {
                    entered.add_permits(1);
                }
                release.acquire_owned().await.unwrap().forget();
                stream::iter([
                    Ok(ModelEvent::text_delta("actual audited result")),
                    Ok(ModelEvent::Finish {
                        reason: ModelFinishReason::Stop,
                    }),
                ])
            })
            .flatten();
            let initial = if partial {
                vec![Ok(ModelEvent::text_delta("real partial before force"))]
            } else {
                Vec::new()
            };
            Ok(Box::pin(stream::iter(initial).chain(tail)) as ModelStream)
        })
    }
}
struct Resources {
    model: Arc<ModelFixture>,
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
fn resources() -> (
    Arc<dyn ResourceResolver>,
    Arc<Semaphore>,
    Arc<Semaphore>,
    Arc<AtomicUsize>,
) {
    resources_with_partial(false)
}
fn resources_with_partial(
    partial: bool,
) -> (
    Arc<dyn ResourceResolver>,
    Arc<Semaphore>,
    Arc<Semaphore>,
    Arc<AtomicUsize>,
) {
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(ModelFixture {
        descriptor: ModelDescriptor::new(
            "audit/model".parse().unwrap(),
            128_000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        entered: entered.clone(),
        release: release.clone(),
        calls: calls.clone(),
        partial,
    });
    (Arc::new(Resources { model }), entered, release, calls)
}
fn settings() -> HostSettings {
    HostSettings {
        name: "independent shutdown audit".into(),
    }
}
fn request(mode: StorageMode) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "audit/model".parse().unwrap();
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "audit Agent".into(),
        mode,
        config,
        default_session: None,
        initial_message: None,
    }
}
fn input(text: &str) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: text.into(),
        in_reply_to: None,
    }
}
async fn until<T>(mut f: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(value) = f() {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("deterministic acceptance condition did not settle")
}
async fn create(host: &Arc<Host>, caller: &CallerContext, mode: StorageMode) -> SessionRef {
    let result = host
        .create_agent_async(caller, request(mode))
        .await
        .unwrap();
    assert!(result.result.steps.agent.succeeded());
    assert!(result.result.steps.session.succeeded());
    SessionRef {
        agent_id: result.agent_id.unwrap(),
        session_id: result.session_id.unwrap(),
    }
}

fn shutdown(
    host: &Arc<Host>,
    caller: &CallerContext,
    strategy: HostShutdownStrategy,
) -> HostShutdownReceipt {
    let request = HostShutdownRequest {
        operation_id: OperationId::new().unwrap(),
        strategy,
    };
    let receipt = host.shutdown(caller, request.clone()).unwrap();
    assert_eq!(receipt.operation_id, request.operation_id);
    receipt.result.validate().unwrap();
    receipt
}
async fn stopped(host: &Host, caller: &CallerContext) {
    until(|| (host.info(caller).unwrap().lifecycle == HostLifecycle::Stopped).then_some(())).await
}
fn history_path(dir: &std::path::Path, target: &SessionRef) -> std::path::PathBuf {
    dir.join("agents")
        .join(target.agent_id.to_string())
        .join("sessions")
        .join(target.session_id.to_string())
        .join("history.jsonl")
}

#[tokio::test]
async fn shutdown_graceful_pause_force_upgrade_release_and_no_persistent_close() {
    let dir = tempfile::tempdir().unwrap();
    let (resources, _, _, calls) = resources();
    let host = Host::open_persistent(dir.path(), settings(), resources.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    let observer = host.session_states(&caller, &target).unwrap();
    host.pause(&caller, &target).unwrap();
    let queued = input("accepted but never consumed");
    host.send(&caller, SendMessage::new(queued.clone(), target.clone()))
        .unwrap();
    let first = shutdown(&host, &caller, HostShutdownStrategy::Graceful);
    assert_eq!(first.result.strategy, HostShutdownStrategy::Graceful);
    assert_eq!(first.result.lifecycle, HostLifecycle::Draining);
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        host.info(&caller).unwrap().lifecycle,
        HostLifecycle::Draining
    );
    let state = host.session_state(&caller, &target).unwrap();
    assert_eq!(state.data.scheduling, Scheduling::Paused);
    assert_eq!(state.data.next_turn.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut forged_reply = input("not an established reply association");
    forged_reply.in_reply_to = Some(queued.message_id);
    assert!(
        host.send(&caller, SendMessage::new(forged_reply, target.clone()))
            .is_err()
    );
    assert!(
        host.create_agent_async(&caller, request(StorageMode::Memory))
            .await
            .is_err()
    );
    let force = shutdown(&host, &caller, HostShutdownStrategy::Force);
    assert_eq!(force.result.strategy, HostShutdownStrategy::Force);
    assert_ne!(force.result.lifecycle, HostLifecycle::Draining);
    let later = shutdown(&host, &caller, HostShutdownStrategy::Graceful);
    assert_eq!(later.result.strategy, HostShutdownStrategy::Force);
    stopped(&host, &caller).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // Neither a live public Host nor an observer is a necessary root owner.
    let reopened = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    let new_connection = reopened.connect().unwrap();
    let new_caller = new_connection.caller();
    assert_eq!(
        reopened
            .session_state(&new_caller, &target)
            .unwrap()
            .data
            .lifecycle,
        SessionLifecycle::Open
    );
    assert!(
        reopened
            .session_state(&new_caller, &target)
            .unwrap()
            .data
            .next_turn
            .is_empty()
    );
    assert!(
        std::fs::read(history_path(dir.path(), &target))
            .unwrap()
            .is_empty()
    );
    assert!(host.resume(&caller, &target).is_err());
    assert!(
        host.close_session(&caller, &target).is_err(),
        "old stopped owner must not create a new close write"
    );
    assert!(host.load_session(&caller, &target).is_err());
    assert!(
        host.send(
            &caller,
            SendMessage::new(input("old owner cannot write"), target.clone())
        )
        .is_err()
    );
    assert!(
        host.create_agent_async(&caller, request(StorageMode::Persistent))
            .await
            .is_err()
    );
    drop(observer);
}

#[tokio::test]
async fn shutdown_force_failed_history_holds_root_until_original_save_continues() {
    let dir = tempfile::tempdir().unwrap();
    let (resources, entered, release, calls) = resources();
    let host = Host::open_persistent(dir.path(), settings(), resources.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    host.send(
        &caller,
        SendMessage::new(input("preserve original range"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    // Metadata reads also open history.jsonl; finish accepted activity I/O
    // before the fixture temporarily removes that path.
    until(|| {
        let state = host.session_state(&caller, &target).unwrap().data;
        assert!(
            state.metadata_fault.is_none(),
            "unexpected activity metadata fault: {:?}",
            state.metadata_fault
        );
        (!state.owned_work.metadata_save).then_some(())
    })
    .await;
    let path = history_path(dir.path(), &target);
    let saved = path.with_extension("audit-original");
    std::fs::rename(&path, &saved).unwrap();
    std::fs::write(&path, b"").unwrap();
    release.add_permits(1);
    let failed = until(|| {
        host.turns(&caller, &target)
            .unwrap()
            .last()
            .filter(|t| {
                t.state == TurnState::Finalizing && t.persistence == PersistenceState::Failed
            })
            .cloned()
    })
    .await;
    let receipt = shutdown(&host, &caller, HostShutdownStrategy::Force);
    assert_eq!(receipt.result.lifecycle, HostLifecycle::Stopping);
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        host.info(&caller).unwrap().lifecycle,
        HostLifecycle::Stopping
    );
    assert!(
        Host::open_persistent(dir.path(), settings(), resources.clone())
            .await
            .is_err()
    );
    // The response has its own activity checkpoint. Finish it before restoring
    // the path so this fixture cannot fail metadata after resume selects work.
    until(|| {
        let state = host.session_state(&caller, &target).unwrap().data;
        assert!(
            state.metadata_fault.is_none(),
            "unexpected activity metadata fault: {:?}",
            state.metadata_fault
        );
        (!state.owned_work.metadata_save).then_some(())
    })
    .await;
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(saved, &path).unwrap();
    host.resume(&caller, &target)
        .expect("resume of original save must remain reachable while stopping");
    stopped(&host, &caller).await;
    let records: Vec<HistoryRecord> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let final_outcomes = records
        .iter()
        .filter_map(|r| match &r.entry {
            HistoryEntry::TurnFinished { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(final_outcomes, vec![failed.outcome.unwrap()]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let reopened = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    let new_connection = reopened.connect().unwrap();
    assert_eq!(
        reopened
            .session_state(&new_connection.caller(), &target)
            .unwrap()
            .data
            .lifecycle,
        SessionLifecycle::Open
    );
}

#[tokio::test]
async fn shutdown_force_cancels_runtime_and_does_not_stop_another_host() {
    let (resources_a, entered_a, _release_a, calls_a) = resources();
    let (resources_b, entered_b, release_b, calls_b) = resources();
    let host_a = Host::memory(resources_a).unwrap();
    let host_b = Host::memory(resources_b).unwrap();
    let connection_a = host_a.connect().unwrap();
    let connection_b = host_b.connect().unwrap();
    let caller_a = connection_a.caller();
    let caller_b = connection_b.caller();
    let target_a = create(&host_a, &caller_a, StorageMode::Memory).await;
    let target_b = create(&host_b, &caller_b, StorageMode::Memory).await;
    host_a
        .send(
            &caller_a,
            SendMessage::new(input("cancel A"), target_a.clone()),
        )
        .unwrap();
    host_b
        .send(
            &caller_b,
            SendMessage::new(input("complete B"), target_b.clone()),
        )
        .unwrap();
    entered_a.acquire().await.unwrap().forget();
    entered_b.acquire().await.unwrap().forget();
    let turn_a = host_a
        .turns(&caller_a, &target_a)
        .unwrap()
        .last()
        .unwrap()
        .id
        .clone();
    let mut abandoned_wait = Box::pin(host_a.wait_turn(&caller_a, &target_a, &turn_a));
    tokio::select! {
        result = &mut abandoned_wait => panic!("unexpected early result: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    shutdown(&host_a, &caller_a, HostShutdownStrategy::Force);
    stopped(&host_a, &caller_a).await;
    assert_eq!(calls_a.load(Ordering::SeqCst), 1);
    assert_eq!(
        host_b.info(&caller_b).unwrap().lifecycle,
        HostLifecycle::Running
    );
    release_b.add_permits(1);
    let turn_b = until(|| {
        host_b
            .turns(&caller_b, &target_b)
            .unwrap()
            .last()
            .filter(|t| t.state == TurnState::Finished)
            .cloned()
    })
    .await;
    assert_eq!(turn_b.outcome, Some(TurnOutcome::Completed));
    assert_eq!(calls_b.load(Ordering::SeqCst), 1);
    drop(abandoned_wait);
}

#[test]
fn shutdown_force_waits_accepted_creation_io_but_does_not_deliver_initial_message() {
    let (runtime, resources, release, calls) = creation_save_gate(1);
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::open_persistent(dir.path(), settings(), resources.clone())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let mut request = request(StorageMode::Persistent);
        request.initial_message = Some(input("must not start after force"));
        let operation = request.operation_id.clone();
        let worker = tokio::spawn({
            let host = host.clone();
            let caller = caller.clone();
            async move { host.create_agent_async(&caller, request).await }
        });
        // An acceptance receipt alone can precede the creation owner's first
        // poll. Wait until that owner has queued its actual save behind the gate.
        resources.wait_for_save().await;
        let receipts = host.creation_receipts(&caller, &operation).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].result.steps.agent, CreateStep::Running);
        assert!(receipts[0].agent_id.is_none());
        let receipt = shutdown(&host, &caller, HostShutdownStrategy::Force);
        assert_eq!(receipt.result.lifecycle, HostLifecycle::Stopping);
        // Let the shutdown owner inspect the pending creation while its real
        // storage work remains blocked, without depending on a yield count.
        resources.wait_for_idle().await;
        assert_eq!(
            host.info(&caller).unwrap().lifecycle,
            HostLifecycle::Stopping
        );
        assert!(!worker.is_finished());
        assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_err());
        release.send(()).unwrap();
        let creation = tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(creation.result.steps.agent.succeeded(), "{creation:?}");
        assert!(!creation.result.steps.message.as_ref().unwrap().succeeded());
        stopped_after_creation(&host, &caller).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_ok());
    });
}

struct CreationSaveGate {
    resources: Arc<dyn ResourceResolver>,
    validations: AtomicUsize,
    block_on_validation: usize,
    blocker_started: Arc<Semaphore>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    park_armed: Arc<AtomicBool>,
    parked: Arc<Semaphore>,
}
impl ResourceResolver for CreationSaveGate {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.resources.validate(config)?;
        if self.validations.fetch_add(1, Ordering::SeqCst) + 1 == self.block_on_validation {
            let release = self.release.lock().unwrap().take().unwrap();
            // Validation runs on the only blocking worker. Queue this fixture
            // before returning, so the next real creation save cannot execute.
            let blocker_started = self.blocker_started.clone();
            tokio::task::spawn_blocking(move || {
                blocker_started.add_permits(1);
                let _ = release.recv();
            });
        }
        Ok(())
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        self.resources.resolve(config)
    }
}
impl CreationSaveGate {
    async fn wait_for_save(&self) {
        // The blocker starts after validation has returned its result. Wait for
        // it before arming the current-thread runtime's next idle boundary.
        tokio::time::timeout(Duration::from_secs(3), self.blocker_started.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        // At idle, the runnable creation has resumed from validation and queued
        // its real save behind the blocker. No fixed sleep/yield count is needed.
        self.wait_for_idle().await;
    }

    async fn wait_for_idle(&self) {
        self.park_armed.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(3), self.parked.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
}
fn creation_save_gate(
    block_on_validation: usize,
) -> (
    tokio::runtime::Runtime,
    Arc<CreationSaveGate>,
    std::sync::mpsc::Sender<()>,
    Arc<AtomicUsize>,
) {
    let park_armed = Arc::new(AtomicBool::new(false));
    let parked = Arc::new(Semaphore::new(0));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .on_thread_park({
            let park_armed = park_armed.clone();
            let parked = parked.clone();
            move || {
                if park_armed.swap(false, Ordering::SeqCst) {
                    parked.add_permits(1);
                }
            }
        })
        .build()
        .unwrap();
    let (resources, _, _, calls) = resources();
    let (release, receiver) = std::sync::mpsc::channel();
    let gate = Arc::new(CreationSaveGate {
        resources,
        validations: AtomicUsize::new(0),
        block_on_validation,
        blocker_started: Arc::new(Semaphore::new(0)),
        release: Mutex::new(Some(receiver)),
        park_armed,
        parked,
    });
    (runtime, gate, release, calls)
}

async fn stopped_after_creation(host: &Host, caller: &CallerContext) {
    let info = until(|| {
        let info = host.info(caller).unwrap();
        (info.lifecycle == HostLifecycle::Stopped || info.shutdown_fault.is_some()).then_some(info)
    })
    .await;
    assert_eq!(info.lifecycle, HostLifecycle::Stopped, "{info:?}");
    assert!(info.shutdown_fault.is_none());
}

fn shutdown_at_default_save(strategy: HostShutdownStrategy, before_publication: bool) {
    // Agent admission validates first; its accepted default Session validates
    // second. A memory default has no Session save before the Agent default save.
    let (runtime, resources, release, calls) = creation_save_gate(2);
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::open_persistent(dir.path(), settings(), resources.clone())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let mut request = request(StorageMode::Persistent);
        request.default_session = Some(SessionOptions {
            mode: Some(StorageMode::Memory),
            overrides: None,
        });
        if before_publication && strategy == HostShutdownStrategy::Force {
            request.initial_message = Some(input("must not start after default save"));
        }
        let operation = request.operation_id.clone();
        let creating = tokio::spawn({
            let host = host.clone();
            let caller = caller.clone();
            async move { host.create_agent_async(&caller, request).await }
        });
        resources.wait_for_save().await;
        let receipts = host.creation_receipts(&caller, &operation).unwrap();
        assert_eq!(resources.validations.load(Ordering::SeqCst), 2);
        assert!(receipts[0].agent_id.is_some());
        assert!(receipts[0].session_id.is_none());
        assert!(receipts[0].result.steps.agent.succeeded());
        assert_eq!(receipts[0].result.steps.session, CreateStep::Running);
        if before_publication {
            shutdown(&host, &caller, strategy);
            assert_ne!(
                host.info(&caller).unwrap().lifecycle,
                HostLifecycle::Stopped
            );
            assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_err());
        }
        release.send(()).unwrap();
        let created = tokio::time::timeout(Duration::from_secs(3), creating)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(created.result.steps.session.succeeded(), "{created:?}");
        if before_publication && strategy == HostShutdownStrategy::Force {
            assert!(matches!(
                created.result.steps.message,
                Some(CreateStep::Failed { ref error })
                    if error.kind() == Some(FaultKind::HostLifecycleUnavailable)
            ));
        }
        if !before_publication {
            shutdown(&host, &caller, strategy);
        }
        stopped_after_creation(&host, &caller).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_ok());
    });
}

#[test]
fn force_during_default_save_reaches_stopped_without_initial_execution() {
    shutdown_at_default_save(HostShutdownStrategy::Force, true);
}

#[test]
fn force_after_default_publication_reaches_stopped() {
    shutdown_at_default_save(HostShutdownStrategy::Force, false);
}

#[test]
fn graceful_during_default_save_reaches_stopped() {
    shutdown_at_default_save(HostShutdownStrategy::Graceful, true);
}

#[test]
fn force_during_existing_agent_session_save_reaches_stopped() {
    // The first two validations create the Agent and its default Session.
    let (runtime, resources, release, calls) = creation_save_gate(3);
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::open_persistent(dir.path(), settings(), resources.clone())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let original = create(&host, &caller, StorageMode::Persistent).await;
        let target = AgentRef {
            host_id: host.info(&caller).unwrap().host_id,
            agent_id: original.agent_id,
        };
        let operation = OperationId::new().unwrap();
        let creating = tokio::spawn({
            let host = host.clone();
            let caller = caller.clone();
            let target = target.clone();
            let operation_id = operation.clone();
            async move {
                host.create_session_async(
                    &caller,
                    SessionCreate {
                        operation_id,
                        target,
                        mode: Some(StorageMode::Persistent),
                        overrides: None,
                        initial_message: Some(input("must not start after Session save")),
                    },
                )
                .await
            }
        });
        resources.wait_for_save().await;
        let pending = host
            .session_creation_receipt(&caller, &target, &operation)
            .unwrap();
        assert_eq!(pending.result.steps.session, CreateStep::Running);
        assert!(pending.session_id.is_none());
        shutdown(&host, &caller, HostShutdownStrategy::Force);
        assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_err());
        release.send(()).unwrap();
        let created = tokio::time::timeout(Duration::from_secs(3), creating)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(created.result.steps.session.succeeded(), "{created:?}");
        assert!(matches!(
            created.result.steps.message,
            Some(CreateStep::Failed { ref error })
                if error.kind() == Some(FaultKind::HostLifecycleUnavailable)
        ));
        stopped_after_creation(&host, &caller).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_ok());
    });
}

#[tokio::test]
async fn shutdown_force_saves_real_partial_and_releases_root_with_abandoned_wait_alive() {
    let dir = tempfile::tempdir().unwrap();
    let (resources, entered, _release, calls) = resources_with_partial(true);
    let host = Host::open_persistent(dir.path(), settings(), resources.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    let observer = host.session_states(&caller, &target).unwrap();
    host.send(
        &caller,
        SendMessage::new(input("retain actual partial"), target.clone()),
    )
    .unwrap();
    // Signal is emitted from the stream's next poll after yielding text_delta,
    // so Runtime has already incorporated the actual partial before shutdown.
    entered.acquire().await.unwrap().forget();
    let turn = host
        .turns(&caller, &target)
        .unwrap()
        .last()
        .unwrap()
        .id
        .clone();
    let mut abandoned_wait = Box::pin(host.wait_turn(&caller, &target, &turn));
    tokio::select! {
        result = &mut abandoned_wait => panic!("unexpected early result: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    shutdown(&host, &caller, HostShutdownStrategy::Force);
    stopped(&host, &caller).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let path = history_path(dir.path(), &target);
    let text = std::fs::read_to_string(&path).unwrap();
    let records: Vec<HistoryRecord> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(text.contains("real partial before force"));
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.entry, HistoryEntry::Partial { .. }))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(
                r.entry,
                HistoryEntry::TurnFinished {
                    outcome: TurnOutcome::Cancelled,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(!records.iter().any(|r| matches!(
        r.entry,
        HistoryEntry::Runtime {
            item: minicore_host_runtime::HistoryItem::Assistant(_),
            ..
        }
    )));
    let reopened = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    assert!(reopened.connect().is_ok());
    // Wait/observer remain intentionally alive through actual lock reacquisition.
    drop(abandoned_wait);
    drop(observer);
}

struct DroppingResources {
    inner: Arc<dyn ResourceResolver>,
    dropped: Arc<AtomicUsize>,
}
impl Drop for DroppingResources {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl ResourceResolver for DroppingResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        self.inner.resolve(config)
    }
}
#[tokio::test]
async fn shutdown_stopped_host_drops_owned_resource_graph_without_public_handle_drop() {
    let (resources, _, _, _) = resources();
    let dropped = Arc::new(AtomicUsize::new(0));
    let host = Host::memory(Arc::new(DroppingResources {
        inner: resources,
        dropped: dropped.clone(),
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let _target = create(&host, &caller, StorageMode::Memory).await;
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    shutdown(&host, &caller, HostShutdownStrategy::Graceful);
    stopped(&host, &caller).await;
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        1,
        "public Host handle still owns execution resource graph after cleanup"
    );
}

#[tokio::test]
async fn graceful_drains_accepted_turns_without_accepting_new_root_work() {
    let (resources, entered, release, calls) = resources();
    let host = Host::memory(resources).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    host.send(
        &caller,
        SendMessage::new(input("first accepted turn"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    host.send(
        &caller,
        SendMessage::new(input("already accepted follow-up"), target.clone()),
    )
    .unwrap();
    shutdown(&host, &caller, HostShutdownStrategy::Graceful);
    assert!(
        host.send(
            &caller,
            SendMessage::new(input("late root input"), target.clone())
        )
        .is_err()
    );
    release.add_permits(1);
    entered.acquire().await.unwrap().forget();
    assert_eq!(
        host.info(&caller).unwrap().lifecycle,
        HostLifecycle::Draining
    );
    release.add_permits(1);
    stopped(&host, &caller).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        host.info(&caller).unwrap().shutdown_strategy,
        Some(HostShutdownStrategy::Graceful)
    );
}

#[tokio::test]
async fn shutdown_rechecks_operator_and_reports_current_facts_for_old_requests() {
    let (resources, _, _, _) = resources();
    let host = Host::memory(resources).unwrap();
    let operator = host.connect().unwrap();
    let reader = host.connect().unwrap();
    let caller = operator.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    host.pause(&caller, &target).unwrap();
    let old = HostShutdownRequest {
        operation_id: OperationId::new().unwrap(),
        strategy: HostShutdownStrategy::Graceful,
    };
    assert_eq!(
        host.shutdown(&reader.caller(), old.clone())
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    let accepted = host.shutdown(&caller, old.clone()).unwrap();
    assert_eq!(accepted.operation_id, old.operation_id);
    let forced = shutdown(&host, &caller, HostShutdownStrategy::Force);
    assert_ne!(forced.operation_id, accepted.operation_id);
    let repeated = host.shutdown(&caller, old.clone()).unwrap();
    assert_eq!(repeated.operation_id, old.operation_id);
    assert_eq!(repeated.result.strategy, HostShutdownStrategy::Force);
    // Empty explicit pause does not itself keep graceful or force alive.
    stopped(&host, &caller).await;
    operator.close();
    assert_eq!(
        host.shutdown(&caller, old.clone()).unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    let replacement = host.connect().unwrap();
    let latest = host.shutdown(&replacement.caller(), old).unwrap();
    assert_eq!(latest.result.lifecycle, HostLifecycle::Stopped);
    assert_eq!(latest.result.strategy, HostShutdownStrategy::Force);
}

#[test]
fn force_keeps_owned_save_and_directory_after_callers_and_host_handle_drop() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let (resources, entered, release_model, calls) = resources();
        let host = Host::open_persistent(dir.path(), settings(), resources)
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let target = create(&host, &caller, StorageMode::Persistent).await;
        host.send(
            &caller,
            SendMessage::new(input("saved even when caller leaves"), target.clone()),
        )
        .unwrap();
        entered.acquire().await.unwrap().forget();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        entered_rx.await.unwrap();
        release_model.add_permits(1);
        until(|| {
            host.session_state(&caller, &target)
                .ok()
                .filter(|snapshot| snapshot.data.saving)
        })
        .await;
        let receipt = shutdown(&host, &caller, HostShutdownStrategy::Force);
        assert_eq!(receipt.result.lifecycle, HostLifecycle::Stopping);
        assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_err());
        let owner = Arc::downgrade(&host);
        drop(connection);
        drop(host);
        assert!(
            owner.upgrade().is_some(),
            "accepted shutdown must retain its owner"
        );
        assert!(minicore_host_storage::DataRoot::acquire(dir.path()).is_err());
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        until(|| minicore_host_storage::DataRoot::acquire(dir.path()).ok()).await;
        until(|| owner.upgrade().is_none().then_some(())).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let records: Vec<HistoryRecord> =
            std::fs::read_to_string(history_path(dir.path(), &target))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.entry, HistoryEntry::TurnFinished { .. }))
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.entry, HistoryEntry::Input { .. }))
                .count(),
            1
        );
    });
}

struct ToolModelFixture {
    descriptor: ModelDescriptor,
    calls: Arc<AtomicUsize>,
}
impl Model for ToolModelFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(Box::pin(stream::iter([
                Ok(ModelEvent::ToolCallStart {
                    tool_call_id: "audit-tool-call".parse().unwrap(),
                    tool_name: "audit_pending".parse().unwrap(),
                }),
                Ok(ModelEvent::tool_call_arguments_delta(
                    "audit-tool-call".parse().unwrap(),
                    "{}",
                )),
                Ok(ModelEvent::ToolCallEnd {
                    tool_call_id: "audit-tool-call".parse().unwrap(),
                }),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                }),
            ])) as ModelStream)
        })
    }
}
struct PendingTool {
    spec: minicore_host_runtime::tools::ToolSpec,
    entered: Arc<Semaphore>,
    cancelled_drop: Arc<AtomicUsize>,
}
struct CancellationDrop {
    cancellation: tokio_util::sync::CancellationToken,
    cancelled_drop: Arc<AtomicUsize>,
}
impl Drop for CancellationDrop {
    fn drop(&mut self) {
        if self.cancellation.is_cancelled() {
            self.cancelled_drop.fetch_add(1, Ordering::SeqCst);
        }
    }
}
impl minicore_host_runtime::tools::Tool for PendingTool {
    fn spec(&self) -> &minicore_host_runtime::tools::ToolSpec {
        &self.spec
    }
    fn execute<'a>(
        &'a self,
        _: minicore_host_runtime::tools::ToolInvocation,
        context: minicore_host_runtime::tools::ToolContext,
    ) -> minicore_host_runtime::tools::ToolFuture<'a> {
        Box::pin(async move {
            let _guard = CancellationDrop {
                cancellation: context.cancellation,
                cancelled_drop: self.cancelled_drop.clone(),
            };
            self.entered.add_permits(1);
            std::future::pending().await
        })
    }
}
struct ToolResources {
    model: Arc<dyn Model>,
    tools: ToolSet,
}
impl ResourceResolver for ToolResources {
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
#[tokio::test]
async fn force_cancels_actual_runtime_tool_and_saves_result_before_root_release() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Semaphore::new(0));
    let cancelled_drop = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(ToolModelFixture {
        descriptor: ModelDescriptor::new(
            "audit/model".parse().unwrap(),
            128_000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap(),
        calls: calls.clone(),
    });
    let mut tools = ToolSet::builder();
    tools.register(PendingTool {
        spec: minicore_host_runtime::tools::ToolSpec::new(
            "audit_pending".parse().unwrap(),
            "Deterministic pending tool fixture",
            serde_json::json!({}),
        )
        .unwrap(),
        entered: entered.clone(),
        cancelled_drop: cancelled_drop.clone(),
    });
    let resources: Arc<dyn ResourceResolver> = Arc::new(ToolResources {
        model,
        tools: tools.build().unwrap(),
    });
    let host = Host::open_persistent(dir.path(), settings(), resources.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    host.send(
        &caller,
        SendMessage::new(input("cancel actual tool"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    shutdown(&host, &caller, HostShutdownStrategy::Force);
    stopped(&host, &caller).await;
    assert_eq!(
        cancelled_drop.load(Ordering::SeqCst),
        1,
        "actual tool future must receive cancellation before release"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "force must prevent the next model request"
    );
    let records: Vec<HistoryRecord> = std::fs::read_to_string(history_path(dir.path(), &target))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 4);
    assert!(
        matches!(&records[2].entry, HistoryEntry::Runtime { item: minicore_host_runtime::HistoryItem::ToolResult(tool), .. }
        if tool.outcome == minicore_host_runtime::tools::ToolResultOutcome::Cancelled)
    );
    assert!(matches!(
        &records[3].entry,
        HistoryEntry::TurnFinished {
            outcome: TurnOutcome::Cancelled,
            ..
        }
    ));
    let reopened = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    assert!(reopened.connect().is_ok());
}
