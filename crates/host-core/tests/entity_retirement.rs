//! Independent acceptance checks: production Host/Journal/Runtime with a
//! deterministic in-process model fixture; no live provider or TUI coverage.
use futures_util::{StreamExt, stream};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
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
                options: LoopOptions::default_checked().unwrap(),
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

#[tokio::test]
async fn closed_memory_sources_disappear_without_observer_retention() {
    let (resources, entered, release, calls) = resources();
    let host = Host::memory(resources).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    let observer = host.session_states(&caller, &target).unwrap();
    let message = input("memory body must not become a tombstone");
    host.send(&caller, SendMessage::new(message.clone(), target.clone()))
        .unwrap();
    entered.acquire().await.unwrap().forget();
    release.add_permits(1);
    let turn = until(|| {
        host.turns(&caller, &target)
            .unwrap()
            .last()
            .filter(|t| t.state == TurnState::Finished)
            .cloned()
    })
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(host.turn_result(&caller, &target, &turn.id).is_ok());
    assert_eq!(
        host.session_state(&caller, &target).unwrap().data.lifecycle,
        SessionLifecycle::Open
    );
    host.close_session(&caller, &target).unwrap();
    // Observer presence cannot keep closed memory registered or its bodies readable.
    until(|| host.session_state(&caller, &target).err()).await;
    assert!(host.turn_result(&caller, &target, &turn.id).is_err());
    assert!(host.message(&caller, &target, &message.message_id).is_err());
    assert!(host.history(&caller, &target).is_err());
    assert!(host.agents(&caller).unwrap().is_empty());
    drop(observer);
}

#[tokio::test]
async fn ordinary_memory_keeps_multiple_turns_and_cancelled_paused_backlog_until_close() {
    let (resources, entered, release, calls) = resources();
    let host = Host::memory(resources).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    host.send(
        &caller,
        SendMessage::new(input("first ordinary turn"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    release.add_permits(1);
    until(|| {
        host.turns(&caller, &target)
            .unwrap()
            .last()
            .filter(|t| t.state == TurnState::Finished)
            .cloned()
    })
    .await;
    assert_eq!(
        host.session_state(&caller, &target).unwrap().data.lifecycle,
        SessionLifecycle::Open
    );
    host.send(
        &caller,
        SendMessage::new(input("second ordinary turn"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    host.pause(&caller, &target).unwrap();
    let queued = input("keep the accepted inbox");
    host.send(&caller, SendMessage::new(queued.clone(), target.clone()))
        .unwrap();
    assert_eq!(
        host.cancel_session(&caller, &target, true).unwrap().status,
        CancelStatus::Claimed
    );
    let cancelled = until(|| {
        host.turns(&caller, &target)
            .unwrap()
            .last()
            .filter(|t| t.state == TurnState::Finished && t.outcome == Some(TurnOutcome::Cancelled))
            .cloned()
    })
    .await;
    assert!(host.turn_result(&caller, &target, &cancelled.id).is_ok());
    let state = host.session_state(&caller, &target).unwrap();
    assert_eq!(state.data.scheduling, Scheduling::Paused);
    assert_eq!(state.data.next_turn.len(), 1);
    assert_eq!(state.data.lifecycle, SessionLifecycle::Open);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let observer = host.session_states(&caller, &target).unwrap();
    host.close_session(&caller, &target).unwrap();
    until(|| host.session_state(&caller, &target).err()).await;
    assert!(observer.snapshot().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(host.agents(&caller).unwrap().is_empty());
}

#[tokio::test]
async fn closing_memory_default_preserves_persistent_agent_without_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, _, _, _) = resources();
    let host = Host::open_persistent(directory.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut create = request(StorageMode::Persistent);
    create.default_session = Some(SessionOptions {
        mode: Some(StorageMode::Memory),
        overrides: None,
    });
    let result = host.create_agent_async(&caller, create).await.unwrap();
    let target = SessionRef {
        agent_id: result.agent_id.unwrap(),
        session_id: result.session_id.unwrap(),
    };
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: target.agent_id.clone(),
    };
    let observer = host.session_states(&caller, &target).unwrap();
    host.close_session(&caller, &target).unwrap();
    until(|| host.session_state(&caller, &target).err()).await;
    assert!(observer.snapshot().is_err());
    assert!(host.sessions(&caller, &agent).unwrap().is_empty());
    assert!(
        host.agent_info(&caller, &agent)
            .unwrap()
            .default_session_id
            .is_none()
    );
    assert_eq!(
        host.agent_info(&caller, &agent).unwrap().lifecycle,
        AgentLifecycle::Active
    );
    let saved: minicore_host_storage::AgentRecord = serde_json::from_slice(
        &std::fs::read(
            directory
                .path()
                .join("agents")
                .join(target.agent_id.to_string())
                .join("agent.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(saved.data.default_session_id.is_none());
    assert!(
        !directory
            .path()
            .join("agents")
            .join(target.agent_id.to_string())
            .join("sessions")
            .join(target.session_id.to_string())
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn last_close_races_new_session_admission_at_the_agent_boundary() {
    for _ in 0..32 {
        let (resources, _, _, _) = resources();
        let host = Host::memory(resources).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let old = create(&host, &caller, StorageMode::Memory).await;
        let agent = AgentRef {
            host_id: host.info(&caller).unwrap().host_id,
            agent_id: old.agent_id.clone(),
        };
        let start = Arc::new(tokio::sync::Barrier::new(2));
        let creation = {
            let host = host.clone();
            let caller = caller.clone();
            let agent = agent.clone();
            let start = start.clone();
            tokio::spawn(async move {
                start.wait().await;
                host.create_session_async(
                    &caller,
                    SessionCreate {
                        operation_id: OperationId::new().unwrap(),
                        target: agent,
                        mode: Some(StorageMode::Memory),
                        overrides: None,
                        initial_message: None,
                    },
                )
                .await
            })
        };
        start.wait().await;
        host.close_session(&caller, &old).unwrap();
        match creation.await.unwrap() {
            Ok(receipt) => {
                assert!(receipt.result.steps.session.succeeded());
                let current = SessionRef {
                    agent_id: agent.agent_id.clone(),
                    session_id: receipt.session_id.unwrap(),
                };
                assert!(host.session_state(&caller, &current).is_ok());
                assert_eq!(
                    host.agent_info(&caller, &agent).unwrap().lifecycle,
                    AgentLifecycle::Active
                );
                host.close_session(&caller, &current).unwrap();
            }
            Err(fault) => assert!(matches!(
                fault.kind(),
                Some(FaultKind::HostAgentNotFound | FaultKind::AgentLifecycleUnavailable)
            )),
        }
        until(|| host.agents(&caller).unwrap().is_empty().then_some(())).await;
    }
}

struct ValidationGate {
    inner: Arc<dyn ResourceResolver>,
    enabled: std::sync::atomic::AtomicBool,
    entered: Semaphore,
    released: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl ResourceResolver for ValidationGate {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        // Deliberate test-only owner-boundary gate, not production I/O in validate.
        if self.enabled.swap(false, Ordering::SeqCst) {
            self.entered.add_permits(1);
            let released = self.released.0.lock().unwrap();
            drop(
                self.released
                    .1
                    .wait_while(released, |released| !*released)
                    .unwrap(),
            );
        }
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        self.inner.resolve(config)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn accepted_creation_prevents_empty_memory_agent_retirement() {
    let (resources, _, _, _) = resources();
    let gate = Arc::new(ValidationGate {
        inner: resources,
        enabled: false.into(),
        entered: Semaphore::new(0),
        released: (false.into(), std::sync::Condvar::new()),
    });
    let host = Host::memory(gate.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let original = create(&host, &caller, StorageMode::Memory).await;
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: original.agent_id.clone(),
    };
    gate.enabled.store(true, Ordering::SeqCst);
    let creation = {
        let host = host.clone();
        let caller = caller.clone();
        let agent = agent.clone();
        tokio::spawn(async move {
            host.create_session_async(
                &caller,
                SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: agent,
                    mode: None,
                    overrides: None,
                    initial_message: None,
                },
            )
            .await
        })
    };
    gate.entered.acquire().await.unwrap().forget();
    host.close_session(&caller, &original).unwrap();
    assert!(host.agent_info(&caller, &agent).is_ok());
    assert!(host.sessions(&caller, &agent).unwrap().is_empty());
    *gate.released.0.lock().unwrap() = true;
    gate.released.1.notify_all();
    let accepted = creation.await.unwrap().unwrap();
    assert!(accepted.result.steps.session.succeeded());
    let target = SessionRef {
        agent_id: agent.agent_id,
        session_id: accepted.session_id.unwrap(),
    };
    assert!(host.session_state(&caller, &target).is_ok());
    host.close_session(&caller, &target).unwrap();
    until(|| host.agents(&caller).unwrap().is_empty().then_some(())).await;
}

#[test]
fn idle_memory_close_reclaims_without_a_tokio_executor() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (resources, _, _, _) = resources();
    let host = Host::memory(resources).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let created = runtime
        .block_on(host.create_agent_async(&caller, request(StorageMode::Memory)))
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    drop(runtime);
    assert!(tokio::runtime::Handle::try_current().is_err());
    let observer = host.session_states(&caller, &target).unwrap();
    host.close_session(&caller, &target).unwrap();
    assert!(host.session_state(&caller, &target).is_err());
    assert!(observer.snapshot().is_err());
    assert!(host.agents(&caller).unwrap().is_empty());
}

#[tokio::test]
async fn host_end_reclaims_memory_bodies_without_closing_persistent_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, entered, release, _) = resources();
    let host = Host::open_persistent(directory.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let persistent = create(&host, &caller, StorageMode::Persistent).await;
    let memory = create(&host, &caller, StorageMode::Memory).await;
    let observer = host.session_states(&caller, &memory).unwrap();
    let message = input("ephemeral body belongs only to this run");
    host.send(&caller, SendMessage::new(message.clone(), memory.clone()))
        .unwrap();
    entered.acquire().await.unwrap().forget();
    release.add_permits(1);
    let turn = until(|| {
        host.turns(&caller, &memory)
            .unwrap()
            .last()
            .filter(|turn| turn.state == TurnState::Finished)
            .cloned()
    })
    .await;
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    until(|| (host.info(&caller).unwrap().lifecycle == HostLifecycle::Stopped).then_some(())).await;
    assert!(observer.snapshot().is_err());
    assert!(host.history(&caller, &memory).is_err());
    assert!(host.message(&caller, &memory, &message.message_id).is_err());
    assert!(host.turn_result(&caller, &memory, &turn.id).is_err());
    assert_eq!(
        host.session_state(&caller, &persistent)
            .unwrap()
            .data
            .lifecycle,
        SessionLifecycle::Open
    );
    assert!(minicore_host_storage::DataRoot::acquire(directory.path()).is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_async_closes_do_not_lose_retirement_handoffs() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, entered, _, calls) = resources();
    let host = Host::open_persistent(directory.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let original = create(&host, &caller, StorageMode::Persistent).await;
    let agent = AgentRef {
        host_id: host.info(&caller).unwrap().host_id,
        agent_id: original.agent_id,
    };
    let mut observations = Vec::new();
    let mut targets = Vec::new();
    for _ in 0..16 {
        let receipt = host
            .create_session_async(
                &caller,
                SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: agent.clone(),
                    mode: Some(StorageMode::Memory),
                    overrides: None,
                    initial_message: None,
                },
            )
            .await
            .unwrap();
        let target = SessionRef {
            agent_id: agent.agent_id.clone(),
            session_id: receipt.session_id.unwrap(),
        };
        observations.push(host.session_states(&caller, &target).unwrap());
        host.send(
            &caller,
            SendMessage::new(input("ordinary active memory work"), target.clone()),
        )
        .unwrap();
        entered.acquire().await.unwrap().forget();
        targets.push(target);
    }
    let mut closes = Vec::new();
    for target in targets {
        let host = host.clone();
        let caller = caller.clone();
        closes.push(tokio::spawn(
            async move { host.close_session(&caller, &target) },
        ));
        tokio::task::yield_now().await;
    }
    for close in closes {
        close.await.unwrap().unwrap();
    }
    until(|| {
        host.sessions(&caller, &agent)
            .ok()
            .filter(|sessions| sessions.len() == 1)
    })
    .await;
    assert!(
        observations
            .iter()
            .all(|observer| observer.snapshot().is_err())
    );
    assert_eq!(calls.load(Ordering::SeqCst), 16);
}
