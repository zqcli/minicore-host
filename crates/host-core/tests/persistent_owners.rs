//! Real temporary files, production owners and real Runtime. The model is an
//! explicitly scripted in-process fixture, not a live provider or TUI test.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_storage::*;
use minicore_host_storage::testing::read_history_text;
use minicore_runtime::{
    ExecutionConfig, HistoryItem, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
};

struct ScriptedModel {
    descriptor: ModelDescriptor,
    seen: Arc<Mutex<Vec<ModelRequest>>>,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.seen.lock().unwrap().push(request);
        Box::pin(async {
            Ok(Box::pin(stream::iter([
                Ok(ModelEvent::text_delta("fixture response")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
struct Resources {
    model: Arc<dyn Model>,
}
impl ResourceResolver for Resources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        if config.model.as_str() == "test/model" {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "fixture model missing",
            ))
        }
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.validate(config)?;
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    Arc::clone(&self.model),
                    RuntimeReasoning::Auto,
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
fn resources() -> (Arc<dyn ResourceResolver>, Arc<Mutex<Vec<ModelRequest>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let model = ScriptedModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            128_000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        seen: Arc::clone(&seen),
    };
    (
        Arc::new(Resources {
            model: Arc::new(model),
        }),
        seen,
    )
}
fn settings(name: &str) -> HostSettings {
    HostSettings { name: name.into() }
}
fn config() -> AgentConfig {
    let mut value = AgentConfig::default();
    value.session_defaults.model = "test/model".parse().unwrap();
    value
}
fn input(text: &str) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: text.into(),
        in_reply_to: None,
    }
}
fn seed(path: &Path) -> (HostId, AgentId, Vec<SessionId>) {
    let root = DataRoot::acquire(path).unwrap();
    let host_id = HostId::new().unwrap();
    let host = root.host(host_id.clone());
    host.save_host(&HostRecord::new(
        host_id.clone(),
        HostConfig::from(settings("saved name")),
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
                name: "saved Agent".into(),
                mode: StorageMode::Persistent,
                config: config(),
                config_revision: 7,
                default_session_id: None,
            },
        ))
        .unwrap();
    let mut ids = Vec::new();
    for _ in 0..2 {
        let id = SessionId::new().unwrap();
        agent
            .session(id.clone(), StorageMode::Persistent)
            .unwrap()
            .save(&SessionRecord::new(
                id.clone(),
                agent_id.clone(),
                SessionData {
                    title: None,
                    mode: StorageMode::Persistent,
                    agent_config_revision: 7,
                    config_revision: 4,
                    effective_config: SessionConfig::from_agent(
                        &config(),
                        &SessionOverrides::default(),
                    )
                    .unwrap(),
                    parent: None,
                    closed: None,
                },
            ))
            .unwrap();
        ids.push(id);
    }
    let mut record = agent.load().unwrap();
    record.data.default_session_id = Some(ids[0].clone());
    agent.save(&record).unwrap();
    (host_id, agent_id, ids)
}
async fn completed(host: &Host, caller: &CallerContext, target: &SessionRef) -> TurnResult {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(turn) = host.turns(caller, target).unwrap().last() {
                return host.wait_turn(caller, target, &turn.id).await.unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn stable_identity_exclusive_root_and_no_implicit_object_creation() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("first"), Arc::clone(&resources))
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let info = host.info(&connection.caller()).unwrap();
    assert!(host.agents(&connection.caller()).unwrap().is_empty());
    assert!(!directory.path().join("agents").exists());
    assert!(
        Host::open_persistent(directory.path(), settings("second"), Arc::clone(&resources))
            .await
            .is_err()
    );
    drop(connection);
    drop(host);
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let next = host.info(&connection.caller()).unwrap();
    assert_eq!(info.host_id, next.host_id);
    assert_ne!(info.host_incarnation, next.host_incarnation);
    assert_eq!(
        host.settings(&connection.caller()).unwrap(),
        Some(settings("first"))
    );
}
#[tokio::test]
async fn discovers_metadata_lazily_and_runs_persistent_history_through_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, seen) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    assert_eq!(host.info(&caller).unwrap().host_id, host_id);
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[0].clone(),
    };
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config_revision,
        4
    );
    assert_eq!(
        host.session_state(&caller, &target).unwrap().data.residency,
        Residency::Unloaded
    );
    assert!(seen.lock().unwrap().is_empty());
    host.send(&caller, SendMessage::new(input("new work"), target.clone()))
        .unwrap();
    let result = completed(&host, &caller, &target).await;
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert_eq!(result.info.outcome, Some(TurnOutcome::Completed));
    let history = read_history_text(
        directory
            .path()
            .join("agents")
            .join(agent_id.as_str())
            .join("sessions")
            .join(ids[0].as_str())
            .join("history.jsonl"),
    )
    .unwrap();
    assert!(history.contains("new work"));
    assert!(history.contains("fixture response"));
    assert!(history.contains("turn_finished"));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(
        host.history(&caller, &target)
            .unwrap()
            .iter()
            .any(|item| matches!(item, HistoryItem::Assistant(_)))
    );
}
#[tokio::test]
async fn corrupt_history_blocks_only_that_session_and_preserves_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let bad = directory
        .path()
        .join("agents")
        .join(agent_id.as_str())
        .join("sessions")
        .join(ids[0].as_str())
        .join("history.jsonl");
    std::fs::write(&bad, b"{broken").unwrap();
    let (resources, seen) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let broken = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[0].clone(),
    };
    let good = SessionRef {
        agent_id,
        session_id: ids[1].clone(),
    };
    host.send(
        &caller,
        SendMessage::new(input("kept queued"), broken.clone()),
    )
    .unwrap();
    host.send(&caller, SendMessage::new(input("healthy"), good.clone()))
        .unwrap();
    completed(&host, &caller, &good).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if host
                .session_state(&caller, &broken)
                .unwrap()
                .data
                .block_reason
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let state = host.session_state(&caller, &broken).unwrap().data;
    assert_eq!(
        state.block_reason.unwrap().fault.unwrap().kind(),
        Some(FaultKind::SessionHistoryLoadFailed)
    );
    assert_eq!(state.next_turn.len(), 1);
    assert!(state.active_turn_id.is_none());
    assert_eq!(std::fs::read(&bad).unwrap(), b"{broken");
    assert_eq!(seen.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn damaged_default_metadata_does_not_hide_healthy_sibling() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let bad = directory
        .path()
        .join("agents")
        .join(agent_id.as_str())
        .join("sessions")
        .join(ids[0].as_str())
        .join("session.json");
    std::fs::write(&bad, b"{bad metadata").unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    assert!(host.agent_load_failures(&caller).unwrap().is_empty());
    assert_eq!(
        host.agent_info(&caller, &agent)
            .unwrap()
            .default_session_id
            .as_ref(),
        Some(&ids[0])
    );
    let failures = host.session_load_failures(&caller, &agent).unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].session_id, ids[0]);
    assert_eq!(
        host.update_agent_async(&caller, &agent, 7, &AgentConfigPatch::default())
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentDefaultSessionUnavailable)
    );
    assert_eq!(
        read_agent(&agent_path(directory.path(), &agent_id))
            .data
            .default_session_id,
        Some(ids[0].clone())
    );
    assert_eq!(
        host.send(
            &caller,
            SendMessage::new(
                input("no redirect"),
                MessageTarget::AgentDefault {
                    host_id: agent.host_id,
                    agent_id: agent_id.clone()
                }
            )
        )
        .unwrap_err()
        .kind(),
        Some(FaultKind::AgentDefaultSessionUnavailable)
    );
    let good = SessionRef {
        agent_id,
        session_id: ids[1].clone(),
    };
    host.send(&caller, SendMessage::new(input("healthy"), good.clone()))
        .unwrap();
    completed(&host, &caller, &good).await;
    assert_eq!(std::fs::read(&bad).unwrap(), b"{bad metadata");
}
#[tokio::test]
async fn unsupported_host_settings_are_not_replaced_with_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("first"), Arc::clone(&resources))
        .await
        .unwrap();
    drop(host);
    let path = directory.path().join("host.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["data"]["config"]["unknown_field"] = true.into();
    let original = serde_json::to_vec(&record).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(
        Host::open_persistent(directory.path(), settings("replacement"), resources)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
fn create_request(mode: StorageMode) -> AgentCreate {
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "created Agent".into(),
        mode,
        config: config(),
        default_session: None,
        initial_message: None,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn persistent_creation_modes_reopen_and_real_partial_input_failure() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("test"), Arc::clone(&resources))
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut request = create_request(StorageMode::Persistent);
    request.initial_message = Some(input(""));
    let created = host.create_agent_async(&caller, request).await.unwrap();
    created.validate_call_return().unwrap();
    assert!(created.result.steps.agent.succeeded());
    assert!(created.result.steps.session.succeeded());
    assert!(matches!(
        created.result.steps.message,
        Some(CreateStep::Failed { .. })
    ));
    let target = AgentRef {
        host_id: created.host_id.clone(),
        agent_id: created.agent_id.clone().unwrap(),
    };
    let persistent = SessionRef {
        agent_id: target.agent_id.clone(),
        session_id: created.session_id.unwrap(),
    };
    let memory = host
        .create_session_async(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: target.clone(),
                mode: Some(StorageMode::Memory),
                overrides: None,
                initial_message: Some(input("memory work")),
            },
        )
        .await
        .unwrap();
    memory.validate_call_return().unwrap();
    let memory_target = SessionRef {
        agent_id: target.agent_id.clone(),
        session_id: memory.session_id.unwrap(),
    };
    completed(&host, &caller, &memory_target).await;
    assert!(
        !directory
            .path()
            .join("agents")
            .join(target.agent_id.as_str())
            .join("sessions")
            .join(memory_target.session_id.as_str())
            .exists()
    );
    let transient = host
        .create_agent_async(&caller, create_request(StorageMode::Memory))
        .await
        .unwrap();
    assert!(
        !directory
            .path()
            .join("agents")
            .join(transient.agent_id.as_ref().unwrap().as_str())
            .exists()
    );
    assert!(
        host.create_session_async(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: AgentRef {
                    host_id: created.host_id,
                    agent_id: transient.agent_id.unwrap()
                },
                mode: Some(StorageMode::Persistent),
                overrides: None,
                initial_message: None
            }
        )
        .await
        .is_err()
    );
    host.send(
        &caller,
        SendMessage::new(input("persistent work"), persistent.clone()),
    )
    .unwrap();
    completed(&host, &caller, &persistent).await;
    drop(connection);
    drop(host);
    wait_for_root_release(directory.path()).await;
    let reopened = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    assert_eq!(reopened.agents(&caller).unwrap().len(), 1);
    assert_eq!(reopened.sessions(&caller, &target).unwrap().len(), 1);
    assert_eq!(
        reopened
            .agent_info(&caller, &target)
            .unwrap()
            .default_session_id,
        Some(persistent.session_id.clone())
    );
    assert!(reopened.turns(&caller, &persistent).unwrap().is_empty());
    host_send_and_check_reopened(&reopened, &caller, &persistent).await;
}
async fn host_send_and_check_reopened(host: &Host, caller: &CallerContext, target: &SessionRef) {
    host.send(
        caller,
        SendMessage::new(input("next incarnation"), target.clone()),
    )
    .unwrap();
    let result = completed(host, caller, target).await;
    assert_eq!(result.info.outcome, Some(TurnOutcome::Completed));
    assert_eq!(
        host.history(caller, target)
            .unwrap()
            .iter()
            .filter(|item| matches!(item, HistoryItem::User(_)))
            .count(),
        2
    );
}
struct GatedValidationFixture {
    inner: Arc<dyn ResourceResolver>,
    count: std::sync::atomic::AtomicUsize,
    entered: tokio::sync::Notify,
    released: (Mutex<bool>, std::sync::Condvar),
}
impl GatedValidationFixture {
    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}
impl ResourceResolver for GatedValidationFixture {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        if self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            self.entered.notify_one();
            let mut released = self.released.0.lock().unwrap();
            while !*released {
                released = self.released.1.wait(released).unwrap();
            }
        }
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        self.inner.resolve(config)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_creation_survives_waiter_abort_and_connection_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let (inner, _) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("test"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut request = create_request(StorageMode::Persistent);
    request.initial_message = Some(input("owned work"));
    let operation = request.operation_id.clone();
    let owner = Arc::clone(&host);
    let task_caller = caller.clone();
    let waiting =
        tokio::spawn(async move { owner.create_agent_async(&task_caller, request).await });
    gate.entered.notified().await;
    let progress = host.receipt(&caller, &operation).unwrap();
    assert!(progress.result.steps.agent.succeeded());
    assert!(progress.result.steps.session.in_progress());
    progress.validate().unwrap();
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::AgentCreate(progress)
    );
    waiting.abort();
    connection.close();
    gate.release();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let value = host.receipt(&caller, &operation).unwrap();
            if value.validate_call_return().is_ok() {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::AgentCreate(receipt.clone())
    );
    assert!(receipt.result.steps.session.succeeded());
    assert!(receipt.result.steps.message.unwrap().succeeded());
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    assert_eq!(
        completed(&host, &caller, &target).await.info.persistence,
        PersistenceState::Persisted
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_session_real_file_failure_keeps_published_agent_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let (inner, _) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("test"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut request = create_request(StorageMode::Persistent);
    request.initial_message = Some(input("not delivered"));
    let operation = request.operation_id.clone();
    let owner = Arc::clone(&host);
    let task_caller = caller.clone();
    let waiting =
        tokio::spawn(async move { owner.create_agent_async(&task_caller, request).await });
    gate.entered.notified().await;
    let agent_id = host.receipt(&caller, &operation).unwrap().agent_id.unwrap();
    std::fs::write(
        directory
            .path()
            .join("agents")
            .join(agent_id.as_str())
            .join("sessions"),
        b"not a directory",
    )
    .unwrap();
    gate.release();
    let receipt = waiting.await.unwrap().unwrap();
    receipt.validate_call_return().unwrap();
    assert_eq!(receipt.agent_id.as_ref(), Some(&agent_id));
    assert!(receipt.session_id.is_none());
    assert!(matches!(
        receipt.result.steps.session,
        CreateStep::Failed { .. }
    ));
    assert_eq!(receipt.result.steps.message, Some(CreateStep::NotAttempted));
    assert!(receipt.message_id.is_some());
    assert!(receipt.result.delivery.is_none());
    assert!(
        directory
            .path()
            .join("agents")
            .join(agent_id.as_str())
            .join("agent.json")
            .exists()
    );
}

#[tokio::test]
async fn review_resume_validates_history_without_new_input() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let bad = directory
        .path()
        .join("agents")
        .join(agent_id.as_str())
        .join("sessions")
        .join(ids[0].as_str())
        .join("history.jsonl");
    std::fs::write(&bad, b"{broken").unwrap();
    let (resources, seen) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    host.pause(&caller, &target).unwrap();
    let result = host.resume(&caller, &target);
    wait_state(&host, &caller, &target, |state| !state.preparing).await;
    let state = host.session_state(&caller, &target).unwrap();
    assert!(seen.lock().unwrap().is_empty());

    assert!(
        result.is_err() || state.data.block_reason.is_some(),
        "resume succeeded without loading or validating corrupted history"
    );
    assert_eq!(state.data.scheduling, Scheduling::Paused);
}

struct ReviewGateModel {
    descriptor: ModelDescriptor,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
    finished: Arc<std::sync::atomic::AtomicBool>,
}
impl Model for ReviewGateModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
            self.finished
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::pin(stream::iter([
                Ok(ModelEvent::text_delta("gate fixture finished")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
#[tokio::test]
async fn review_root_guard_outlives_active_memory_session() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let resources: Arc<dyn ResourceResolver> = Arc::new(Resources {
        model: Arc::new(ReviewGateModel {
            descriptor: ModelDescriptor::new(
                "test/model".parse().unwrap(),
                128_000,
                BTreeSet::from([RuntimeReasoning::Auto]),
                true,
            )
            .unwrap(),
            entered: entered.clone(),
            release: release.clone(),
            finished: finished.clone(),
        }),
    });
    let host = Host::open_persistent(directory.path(), settings("first"), resources.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let receipt = host
        .create_agent_async(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "memory".into(),
                mode: StorageMode::Memory,
                config: config(),
                default_session: None,
                initial_message: Some(input("keep working")),
            },
        )
        .await
        .unwrap();
    assert!(receipt.session_id.is_some());
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert!(!finished.load(Ordering::SeqCst));
    drop(connection);
    drop(host);
    let second = Host::open_persistent(directory.path(), settings("second"), resources).await;
    eprintln!(
        "second Host opened with old memory model still running: {}",
        second.is_ok()
    );
    release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !finished.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        second.is_err(),
        "root ownership ended before accepted memory execution ended"
    );
}

async fn wait_state(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    predicate: impl Fn(&SessionStateData) -> bool,
) -> SessionStateData {
    let mut changes = host.session_states(caller, target).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = changes.snapshot().unwrap().data;
            if predicate(&state) {
                return state;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}
fn agent_path(directory: &Path, agent: &AgentId) -> std::path::PathBuf {
    directory.join("agents").join(agent.as_str())
}
fn session_path(directory: &Path, target: &SessionRef) -> std::path::PathBuf {
    agent_path(directory, &target.agent_id)
        .join("sessions")
        .join(target.session_id.as_str())
}
fn read_agent(path: &Path) -> AgentRecord {
    serde_json::from_slice(&std::fs::read(path.join("agent.json")).unwrap()).unwrap()
}
fn read_session(path: &Path) -> SessionRecord {
    serde_json::from_slice(&std::fs::read(path.join("session.json")).unwrap()).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn durable_config_versions_defaults_and_reload_preserve_real_history() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(
        directory.path(),
        settings("ignored"),
        Arc::clone(&resources),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[0].clone(),
    };
    let role = Role {
        name: "updated".into(),
        description: String::new(),
        instructions: "next session default".into(),
    };
    let updated = host
        .update_agent_async(
            &caller,
            &agent,
            7,
            &AgentConfigPatch {
                role: Some(role.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.config_revision, 8);
    assert_eq!(
        read_agent(&agent_path(directory.path(), &agent_id))
            .data
            .config
            .role,
        role
    );
    assert_ne!(
        host.session_info(&caller, &target).unwrap().config.role,
        role
    );
    let created = host
        .create_session_async(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let new_target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: created.session_id.unwrap(),
    };
    let info = host.session_info(&caller, &new_target).unwrap();
    assert_eq!(info.config.role, role);
    assert_eq!(info.agent_config_revision, 8);
    let updated = host
        .update_session_async(
            &caller,
            &target,
            4,
            &SessionConfigPatch {
                role: Some(role.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.config_revision, 5);
    assert_eq!(
        read_session(&session_path(directory.path(), &target))
            .data
            .effective_config
            .role,
        role
    );
    host.send(
        &caller,
        SendMessage::new(input("before unload"), target.clone()),
    )
    .unwrap();
    completed(&host, &caller, &target).await;
    host.unload(&caller, &target).unwrap();
    host.load_session(&caller, &target).unwrap();
    wait_state(&host, &caller, &target, |state| {
        state.residency == Residency::Loaded && !state.preparing
    })
    .await;
    assert_eq!(
        host.history(&caller, &target)
            .unwrap()
            .iter()
            .filter(|item| matches!(item, HistoryItem::User(_)))
            .count(),
        1
    );
    host.send(
        &caller,
        SendMessage::new(input("after reload"), target.clone()),
    )
    .unwrap();
    wait_state(&host, &caller, &target, |state| {
        state.last_result.is_some()
            && state.active_turn_id.is_none()
            && state.next_turn.is_empty()
            && !state.preparing
    })
    .await;
    assert_eq!(
        host.history(&caller, &target)
            .unwrap()
            .iter()
            .filter(|item| matches!(item, HistoryItem::User(_)))
            .count(),
        2
    );
    drop(connection);
    drop(host);
    wait_for_root_release(directory.path()).await;
    let reopened = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    assert_eq!(
        reopened
            .agent_info(&caller, &agent)
            .unwrap()
            .config_revision,
        8
    );
    assert_eq!(
        reopened
            .session_info(&caller, &target)
            .unwrap()
            .config_revision,
        5
    );
    assert!(reopened.turns(&caller, &target).unwrap().is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_write_failure_retains_old_config_and_reconciles_only_original_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    let path = session_path(directory.path(), &target);
    let original = host.session_info(&caller, &target).unwrap();
    std::fs::rename(path.join("session.json"), path.join("original.json")).unwrap();
    std::fs::create_dir(path.join("session.json")).unwrap();
    let role = Role {
        name: "candidate".into(),
        description: String::new(),
        instructions: "new role".into(),
    };
    assert!(
        host.update_session_async(
            &caller,
            &target,
            4,
            &SessionConfigPatch {
                role: Some(role.clone()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config,
        original.config
    );
    assert!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .metadata_fault
            .is_some()
    );
    assert!(
        host.update_session_async(
            &caller,
            &target,
            4,
            &SessionConfigPatch {
                role: Some(Role::default()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    std::fs::remove_dir(path.join("session.json")).unwrap();
    std::fs::rename(path.join("original.json"), path.join("session.json")).unwrap();
    let original_bytes = std::fs::read(path.join("session.json")).unwrap();
    let mut unrelated = read_session(&path);
    unrelated.data.effective_config.role.name = "unrelated external value".into();
    let unrelated_bytes = serde_json::to_vec(&unrelated).unwrap();
    std::fs::write(path.join("session.json"), &unrelated_bytes).unwrap();
    assert!(
        host.reconcile_session_record(&caller, &target)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(path.join("session.json")).unwrap(),
        unrelated_bytes
    );
    std::fs::write(path.join("session.json"), original_bytes).unwrap();
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config_revision,
        4
    );
    let saved = host
        .reconcile_session_record(&caller, &target)
        .await
        .unwrap();
    assert_eq!(saved.config_revision, 5);
    assert_eq!(saved.config.role, role);
    assert_eq!(read_session(&path).data.effective_config.role, role);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn close_persists_default_clear_before_closed_and_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(
        directory.path(),
        settings("ignored"),
        Arc::clone(&resources),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[0].clone(),
    };
    host.close_session(&caller, &target).unwrap();
    assert!(
        host.send(&caller, SendMessage::new(input("rejected"), target.clone()))
            .is_err()
    );
    wait_state(&host, &caller, &target, |state| {
        state.lifecycle == SessionLifecycle::Closed
    })
    .await;
    assert_eq!(
        read_agent(&agent_path(directory.path(), &agent_id))
            .data
            .default_session_id,
        None
    );
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_some()
    );
    assert!(
        session_path(directory.path(), &target)
            .join("history.jsonl")
            .exists()
    );
    assert!(
        host.set_default_session_async(&caller, &agent, Some(target.session_id.clone()))
            .await
            .is_err()
    );
    drop(connection);
    drop(host);
    // The direct close owner may still be releasing its last stack-local lease.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(root) = DataRoot::acquire(directory.path()) {
                drop(root);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reopened = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    assert_eq!(
        reopened
            .session_state(&caller, &target)
            .unwrap()
            .data
            .lifecycle,
        SessionLifecycle::Closed
    );
    assert_eq!(
        reopened
            .agent_info(&caller, &agent)
            .unwrap()
            .default_session_id,
        None
    );
    assert_eq!(
        reopened.resume(&caller, &target).unwrap_err().kind(),
        Some(FaultKind::SessionLifecycleClosed)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_default_clear_keeps_session_closing_until_exact_save_is_reconciled() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[0].clone(),
    };
    let path = agent_path(directory.path(), &agent_id);
    std::fs::rename(path.join("agent.json"), path.join("original.json")).unwrap();
    std::fs::create_dir(path.join("agent.json")).unwrap();
    host.close_session(&caller, &target).unwrap();
    let state = wait_state(&host, &caller, &target, |state| {
        state.metadata_fault.is_some()
    })
    .await;
    assert_eq!(state.lifecycle, SessionLifecycle::Closing);
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_none()
    );
    std::fs::remove_dir(path.join("agent.json")).unwrap();
    std::fs::rename(path.join("original.json"), path.join("agent.json")).unwrap();
    let original_bytes = std::fs::read(path.join("agent.json")).unwrap();
    let mut unrelated = read_agent(&path);
    unrelated.data.default_session_id = Some(ids[1].clone());
    let unrelated_bytes = serde_json::to_vec(&unrelated).unwrap();
    std::fs::write(path.join("agent.json"), &unrelated_bytes).unwrap();
    assert!(host.reconcile_agent_record(&caller, &agent).await.is_err());
    assert_eq!(
        std::fs::read(path.join("agent.json")).unwrap(),
        unrelated_bytes
    );
    std::fs::write(path.join("agent.json"), original_bytes).unwrap();
    assert_eq!(
        read_agent(&path).data.default_session_id,
        Some(target.session_id.clone())
    );
    host.reconcile_agent_record(&caller, &agent).await.unwrap();
    assert_eq!(read_agent(&path).data.default_session_id, None);
    host.close_session(&caller, &target).unwrap();
    wait_state(&host, &caller, &target, |state| {
        state.lifecycle == SessionLifecycle::Closed
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn memory_default_is_runtime_only_and_never_recreated_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("test"), Arc::clone(&resources))
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut request = create_request(StorageMode::Persistent);
    request.default_session = Some(SessionOptions {
        mode: Some(StorageMode::Memory),
        overrides: None,
    });
    let receipt = host.create_agent_async(&caller, request).await.unwrap();
    receipt.validate_call_return().unwrap();
    let agent_id = receipt.agent_id.unwrap();
    assert_eq!(
        read_agent(&agent_path(directory.path(), &agent_id))
            .data
            .default_session_id,
        None
    );
    assert!(
        !agent_path(directory.path(), &agent_id)
            .join("sessions")
            .exists()
    );
    drop(connection);
    drop(host);
    let reopened = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id: receipt.host_id,
        agent_id,
    };
    assert!(reopened.sessions(&caller, &agent).unwrap().is_empty());
    assert_eq!(
        reopened
            .agent_info(&caller, &agent)
            .unwrap()
            .default_session_id,
        None
    );
}

#[tokio::test]
async fn review_check_closed_session_cannot_reopen_or_write() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    {
        let root = DataRoot::acquire(directory.path()).unwrap();
        let store = root
            .host(host_id)
            .agent(agent_id.clone(), StorageMode::Persistent)
            .session(ids[0].clone(), StorageMode::Persistent)
            .unwrap();
        let mut record = store.load().unwrap();
        record.data.closed = Some(SessionClosure {
            reason: "finished ownership".into(),
        });
        store.save(&record).unwrap();
    }
    let path = directory
        .path()
        .join("agents")
        .join(agent_id.as_str())
        .join("sessions")
        .join(ids[0].as_str())
        .join("history.jsonl");
    let original = std::fs::read(&path).unwrap();
    let (resources, seen) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    assert_eq!(
        host.session_state(&caller, &target).unwrap().data.lifecycle,
        SessionLifecycle::Closed
    );
    assert_eq!(
        host.resume(&caller, &target).unwrap_err().kind(),
        Some(FaultKind::SessionLifecycleClosed)
    );
    assert_eq!(
        host.send(&caller, SendMessage::new(input("do not execute"), target))
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionLifecycleClosed)
    );
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

fn review_gate_resources() -> (
    Arc<dyn ResourceResolver>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Semaphore>,
) {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let resources: Arc<dyn ResourceResolver> = Arc::new(Resources {
        model: Arc::new(ReviewGateModel {
            descriptor: ModelDescriptor::new(
                "test/model".parse().unwrap(),
                128_000,
                BTreeSet::from([RuntimeReasoning::Auto]),
                true,
            )
            .unwrap(),
            entered: entered.clone(),
            release: release.clone(),
            finished: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }),
    });
    (resources, entered, release)
}

#[tokio::test]
async fn review_check_active_persistent_session_retains_root_guard() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let (resources, entered, release) = review_gate_resources();
    let host = Host::open_persistent(directory.path(), settings("first"), resources.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    host.send(
        &connection.caller(),
        SendMessage::new(input("keep working"), target),
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    drop(connection);
    drop(host);
    assert!(
        Host::open_persistent(directory.path(), settings("second"), resources)
            .await
            .is_err()
    );
    release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(root) = DataRoot::acquire(directory.path()) {
                drop(root);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn review_check_real_save_failure_preserves_completed_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let path = directory
        .path()
        .join("agents")
        .join(agent_id.as_str())
        .join("sessions")
        .join(ids[0].as_str())
        .join("history.jsonl");
    let (resources, entered, release) = review_gate_resources();
    let host = Host::open_persistent(directory.path(), settings("first"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    host.send(
        &caller,
        SendMessage::new(input("keep completed"), target.clone()),
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    std::fs::rename(&path, path.with_extension("jsonl.saved")).unwrap();
    std::fs::write(&path, b"").unwrap();
    release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(turn) = host.turns(&caller, &target).unwrap().last()
                && turn.persistence == PersistenceState::Failed
                && turn.outcome.is_some()
            {
                assert_eq!(turn.state, TurnState::Finalizing);
                assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
                assert!(turn.fault.is_none());
                assert_eq!(
                    turn.persistence_fault.as_ref().unwrap().kind(),
                    Some(FaultKind::SessionHistoryWriteFailed)
                );
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    assert!(host.load_session(&caller, &target).is_err());
    assert!(host.unload(&caller, &target).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    assert!(
        host.history(&caller, &target)
            .unwrap()
            .iter()
            .any(|item| matches!(item, HistoryItem::Assistant(_)))
    );
}

#[tokio::test]
async fn review_check_restart_uses_saved_context_with_fresh_turn_and_load() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    let (r1, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("first"), r1)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let old_host = host.info(&caller).unwrap();
    host.send(
        &caller,
        SendMessage::new(input("old unique prompt"), target.clone()),
    )
    .unwrap();
    let old = completed(&host, &caller, &target).await;
    drop(caller);
    drop(connection);
    drop(host);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(root) = DataRoot::acquire(directory.path()) {
                drop(root);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (r2, seen) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), r2)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    assert_eq!(host.info(&caller).unwrap().host_id, old_host.host_id);
    assert_ne!(
        host.info(&caller).unwrap().host_incarnation,
        old_host.host_incarnation
    );
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    host.send(
        &caller,
        SendMessage::new(input("new unique prompt"), target.clone()),
    )
    .unwrap();
    let new = completed(&host, &caller, &target).await;
    assert_ne!(new.info.id, old.info.id);
    assert_ne!(new.info.load_id, old.info.load_id);
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let serialized = serde_json::to_string(&requests[0]).unwrap();
    assert!(serialized.contains("old unique prompt"));
    assert!(serialized.contains("new unique prompt"));
    assert!(serialized.contains("fixture response"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_single_unsafe_session_directory_is_isolated() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let bad = directory
        .path()
        .join("agents")
        .join(agent_id.as_str())
        .join("sessions")
        .join(ids[0].as_str());
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o777)).unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("ignored"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[1].clone(),
    };
    let child_result = host.session_info(&caller, &target);
    let failures = host.agent_load_failures(&caller).unwrap();
    assert!(failures.is_empty());
    assert_eq!(
        host.session_load_failures(
            &caller,
            &AgentRef {
                host_id: host_id.clone(),
                agent_id: agent_id.clone()
            }
        )
        .unwrap()[0]
            .session_id,
        ids[0]
    );
    assert!(
        child_result.is_ok(),
        "single child directory fault hid healthy sibling and Agent {}",
        host_id
    );
}

#[test]
fn accepted_metadata_saves_outlive_both_waiters_and_connection() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let (host_id, agent_id, ids) = seed(directory.path());
        let (resources, _) = resources();
        let host = Host::open_persistent(directory.path(), settings("test"), resources)
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let agent = AgentRef {
            host_id,
            agent_id: agent_id.clone(),
        };
        let target = SessionRef {
            agent_id: agent_id.clone(),
            session_id: ids[0].clone(),
        };
        // A scheduling fixture occupies the blocking pool. The actual metadata
        // work still uses production storage against real temporary files.
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, entering) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = held.recv();
        });
        entering.await.unwrap();
        let owner = Arc::clone(&host);
        let task_caller = caller.clone();
        let task_target = target.clone();
        let session_waiter = tokio::spawn(async move {
            owner
                .update_session_async(
                    &task_caller,
                    &task_target,
                    4,
                    &SessionConfigPatch {
                        role: Some(Role {
                            name: "accepted Session config".into(),
                            ..Role::default()
                        }),
                        ..Default::default()
                    },
                )
                .await
        });
        let owner = Arc::clone(&host);
        let task_caller = caller.clone();
        let task_target = agent.clone();
        let agent_waiter = tokio::spawn(async move {
            owner
                .update_agent_async(
                    &task_caller,
                    &task_target,
                    7,
                    &AgentConfigPatch {
                        role: Some(Role {
                            name: "accepted Agent config".into(),
                            ..Role::default()
                        }),
                        ..Default::default()
                    },
                )
                .await
        });
        wait_state(&host, &caller, &target, |state| state.metadata_saving).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !host.agent_info(&caller, &agent).unwrap().metadata_saving {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            host.session_info(&caller, &target).unwrap().config_revision,
            4
        );
        assert_eq!(host.agent_info(&caller, &agent).unwrap().config_revision, 7);
        session_waiter.abort();
        agent_waiter.abort();
        assert!(session_waiter.await.unwrap_err().is_cancelled());
        assert!(agent_waiter.await.unwrap_err().is_cancelled());
        connection.close();
        let observer = host.connect().unwrap();
        let caller = observer.caller();
        release.send(()).unwrap();
        blocker.await.unwrap();
        wait_state(&host, &caller, &target, |state| !state.metadata_saving).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while host.agent_info(&caller, &agent).unwrap().metadata_saving {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let saved_session = read_session(&session_path(directory.path(), &target));
        let saved_agent = read_agent(&agent_path(directory.path(), &agent_id));
        assert_eq!(saved_session.data.config_revision, 5);
        assert_eq!(
            saved_session.data.effective_config.role.name,
            "accepted Session config"
        );
        assert_eq!(saved_agent.data.config_revision, 8);
        assert_eq!(saved_agent.data.config.role.name, "accepted Agent config");
        host.send(
            &caller,
            SendMessage::new(input("after owned saves"), target.clone()),
        )
        .unwrap();
        assert_eq!(
            completed(&host, &caller, &target).await.info.persistence,
            PersistenceState::Persisted
        );
    });
}

#[test]
fn later_pause_wins_over_accepted_resume_history_load() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let (_, agent_id, ids) = seed(directory.path());
        let (resources, seen) = resources();
        let host = Host::open_persistent(directory.path(), settings("test"), resources)
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let target = SessionRef {
            agent_id,
            session_id: ids[0].clone(),
        };
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, entering) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = held.recv();
        });
        entering.await.unwrap();
        host.pause(&caller, &target).unwrap();
        host.resume(&caller, &target).unwrap();
        wait_state(&host, &caller, &target, |state| state.preparing).await;
        host.pause(&caller, &target).unwrap();
        host.send(
            &caller,
            SendMessage::new(input("stay queued after later pause"), target.clone()),
        )
        .unwrap();
        release.send(()).unwrap();
        blocker.await.unwrap();
        let loaded = wait_state(&host, &caller, &target, |state| !state.preparing).await;
        assert_eq!(loaded.residency, Residency::Loaded);
        assert_eq!(loaded.scheduling, Scheduling::Paused);
        assert_eq!(loaded.next_turn.len(), 1);
        assert!(seen.lock().unwrap().is_empty());
        host.resume(&caller, &target).unwrap();
        assert_eq!(
            completed(&host, &caller, &target).await.info.persistence,
            PersistenceState::Persisted
        );
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_create_reused_operation_does_not_mix_owner_or_message() {
    let directory = tempfile::tempdir().unwrap();
    let (inner, _) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("test"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut first = create_request(StorageMode::Persistent);
    first.initial_message = Some(input("first independent invocation"));
    let operation = first.operation_id.clone();
    let first_message = first.initial_message.as_ref().unwrap().message_id.clone();
    let owner = host.clone();
    let first_caller = caller.clone();
    let waiting = tokio::spawn(async move { owner.create_agent_async(&first_caller, first).await });
    gate.entered.notified().await;
    let first_agent = host.receipt(&caller, &operation).unwrap().agent_id.unwrap();
    let mut second = create_request(StorageMode::Persistent);
    second.operation_id = operation.clone();
    second.initial_message = Some(input("second independent invocation"));
    let second_receipt = host.create_agent_async(&caller, second).await.unwrap();
    gate.release();
    let first_receipt = waiting.await.unwrap().unwrap();
    first_receipt.validate_call_return().unwrap();
    second_receipt.validate_call_return().unwrap();
    let retained = host.creation_receipts(&caller, &operation).unwrap();
    assert_eq!(retained.len(), 2);
    assert!(retained.contains(&first_receipt));
    assert!(retained.contains(&second_receipt));
    assert_eq!(
        host.receipt(&caller, &operation).unwrap_err().kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );

    assert_ne!(first_agent, second_receipt.agent_id.clone().unwrap());
    assert_eq!(
        first_receipt.agent_id.as_ref(),
        Some(&first_agent),
        "first creation returned second invocation's Agent"
    );
    assert_eq!(
        first_receipt.message_id.as_ref(),
        Some(&first_message),
        "first creation returned second invocation's MessageId"
    );
    let target = SessionRef {
        agent_id: first_receipt.agent_id.unwrap(),
        session_id: first_receipt.session_id.unwrap(),
    };
    assert!(
        host.session_info(&caller, &target).is_ok(),
        "receipt must contain a valid direct-owner pair"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_create_check_direct_session_survives_abandoned_waiter() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, _) = seed(directory.path());
    let (inner, _) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(1),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("test"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = AgentRef { host_id, agent_id };
    let operation = OperationId::new().unwrap();
    let request = SessionCreate {
        operation_id: operation.clone(),
        target: target.clone(),
        mode: None,
        overrides: None,
        initial_message: Some(input("standalone owned creation")),
    };
    let owner = host.clone();
    let child_caller = caller.clone();
    let waiting =
        tokio::spawn(async move { owner.create_session_async(&child_caller, request).await });
    gate.entered.notified().await;
    let pending = host
        .session_creation_receipt(&caller, &target, &operation)
        .unwrap();
    pending.validate().unwrap();
    assert!(pending.result.steps.session.in_progress());
    waiting.abort();
    connection.close();
    gate.release();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = host
                .session_creation_receipt(&caller, &target, &operation)
                .unwrap();
            if receipt.validate_call_return().is_ok() {
                break receipt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(receipt.result.steps.session.succeeded());
    assert!(receipt.result.steps.message.unwrap().succeeded());
    let created = SessionRef {
        agent_id: target.agent_id,
        session_id: receipt.session_id.unwrap(),
    };
    assert_eq!(
        completed(&host, &caller, &created).await.info.persistence,
        PersistenceState::Persisted
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_create_check_stop_waits_for_accepted_creation() {
    let directory = tempfile::tempdir().unwrap();
    let (inner, seen) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("test"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut request = create_request(StorageMode::Persistent);
    request.initial_message = Some(input("must be refused after stop"));
    let operation = request.operation_id.clone();
    let owner = host.clone();
    let child_caller = caller.clone();
    let waiting =
        tokio::spawn(async move { owner.create_agent_async(&child_caller, request).await });
    gate.entered.notified().await;
    let initial = host.receipt(&caller, &operation).unwrap();
    let target = AgentRef {
        host_id: initial.host_id,
        agent_id: initial.agent_id.unwrap(),
    };
    let stop = host.stop_agent(&caller, &target);
    let during = host.agent_info(&caller, &target).unwrap().lifecycle;
    let early_start = host.start_agent(&caller, &target);
    gate.release();
    stop.unwrap();
    assert_eq!(during, AgentLifecycle::Stopping);
    assert!(early_start.is_err());
    let receipt = waiting.await.unwrap().unwrap();
    receipt.validate_call_return().unwrap();
    assert!(receipt.result.steps.agent.succeeded());
    assert!(receipt.result.steps.session.succeeded());
    assert!(matches!(
        receipt.result.steps.message,
        Some(CreateStep::Failed { .. })
    ));
    assert!(receipt.result.delivery.is_none());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while host.agent_info(&caller, &target).unwrap().lifecycle != AgentLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(seen.lock().unwrap().is_empty());
    let created = SessionRef {
        agent_id: target.agent_id.clone(),
        session_id: receipt.session_id.unwrap(),
    };
    assert!(
        host.session_state(&caller, &created)
            .unwrap()
            .data
            .next_turn
            .is_empty()
    );
    host.start_agent(&caller, &target).unwrap();
    host.send(
        &caller,
        SendMessage::new(input("new explicit input after start"), created.clone()),
    )
    .unwrap();
    assert_eq!(
        completed(&host, &caller, &created).await.info.outcome,
        Some(TurnOutcome::Completed)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_create_check_selected_config_version_is_not_reread() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, _) = seed(directory.path());
    let (inner, _) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(1),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("test"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = AgentRef { host_id, agent_id };
    let selected = host.agent_info(&caller, &target).unwrap();
    let request = SessionCreate {
        operation_id: OperationId::new().unwrap(),
        target: target.clone(),
        mode: None,
        overrides: None,
        initial_message: None,
    };
    let owner = host.clone();
    let child_caller = caller.clone();
    let waiting =
        tokio::spawn(async move { owner.create_session_async(&child_caller, request).await });
    gate.entered.notified().await;
    let next = Role {
        instructions: "future version eight".into(),
        ..Default::default()
    };
    let update = host
        .update_agent_async(
            &caller,
            &target,
            7,
            &AgentConfigPatch {
                role: Some(next),
                session_defaults: None,
            },
        )
        .await;
    gate.release();
    assert_eq!(update.unwrap().config_revision, 8);
    let receipt = waiting.await.unwrap().unwrap();
    let first = SessionRef {
        agent_id: target.agent_id.clone(),
        session_id: receipt.session_id.unwrap(),
    };
    let first_info = host.session_info(&caller, &first).unwrap();
    assert_eq!(first_info.config.role, selected.config.role);
    assert_eq!(first_info.agent_config_revision, 7);
    let new = host
        .create_session_async(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: target.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let second = SessionRef {
        agent_id: target.agent_id,
        session_id: new.session_id.unwrap(),
    };
    assert_eq!(
        host.session_info(&caller, &second)
            .unwrap()
            .config
            .role
            .instructions,
        "future version eight"
    );
    assert_eq!(
        host.session_info(&caller, &second)
            .unwrap()
            .agent_config_revision,
        8
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_create_check_distinct_invocations_keep_actual_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("test"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut tasks = Vec::new();
    for i in 0..16 {
        let mut request = create_request(StorageMode::Persistent);
        request.initial_message = Some(input(&format!("independent request {i}")));
        let operation = request.operation_id.clone();
        let message = request.initial_message.as_ref().unwrap().message_id.clone();
        let owner = host.clone();
        let child_caller = caller.clone();
        tasks.push(tokio::spawn(async move {
            (
                operation,
                message,
                owner
                    .create_agent_async(&child_caller, request)
                    .await
                    .unwrap(),
            )
        }));
    }
    let mut agents = BTreeSet::new();
    for task in tasks {
        let (operation, message, receipt) = task.await.unwrap();
        receipt.validate_call_return().unwrap();
        assert_eq!(receipt.operation_id, operation);
        assert_eq!(receipt.message_id.as_ref(), Some(&message));
        assert_eq!(host.receipt(&caller, &operation).unwrap(), receipt);
        let target = SessionRef {
            agent_id: receipt.agent_id.unwrap(),
            session_id: receipt.session_id.unwrap(),
        };
        assert!(agents.insert(target.agent_id.clone()));
        assert_eq!(
            host.session_info(&caller, &target).unwrap().agent_id,
            target.agent_id
        );
        assert_eq!(
            completed(&host, &caller, &target).await.info.persistence,
            PersistenceState::Persisted
        );
    }
    assert_eq!(agents.len(), 16);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_create_reused_session_operation_keeps_actual_message() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, _) = seed(directory.path());
    let (inner, _) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(1),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("test"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = AgentRef { host_id, agent_id };
    let operation = OperationId::new().unwrap();
    let first_input = input("first Session creation");
    let first_message = first_input.message_id.clone();
    let first = SessionCreate {
        operation_id: operation.clone(),
        target: target.clone(),
        mode: None,
        overrides: None,
        initial_message: Some(first_input),
    };
    let owner = host.clone();
    let child_caller = caller.clone();
    let waiting =
        tokio::spawn(async move { owner.create_session_async(&child_caller, first).await });
    gate.entered.notified().await;
    let second = SessionCreate {
        operation_id: operation.clone(),
        target: target.clone(),
        mode: None,
        overrides: None,
        initial_message: Some(input("second Session creation")),
    };
    let second_receipt = host.create_session_async(&caller, second).await.unwrap();
    gate.release();
    let first_receipt = waiting.await.unwrap().unwrap();
    first_receipt.validate_call_return().unwrap();
    second_receipt.validate_call_return().unwrap();
    let retained = host
        .session_creation_receipts(&caller, &target, &operation)
        .unwrap();
    assert_eq!(retained.len(), 2);
    assert!(retained.contains(&first_receipt));
    assert!(retained.contains(&second_receipt));
    assert_eq!(
        host.session_creation_receipt(&caller, &target, &operation)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );

    assert_ne!(first_receipt.session_id, second_receipt.session_id);
    let created = SessionRef {
        agent_id: first_receipt.agent_id,
        session_id: first_receipt.session_id.unwrap(),
    };
    assert!(host.message(&caller, &created, &first_message).is_ok());
    assert_eq!(
        first_receipt.message_id.as_ref(),
        Some(&first_message),
        "receipt claimed acceptance of the other invocation's MessageId"
    );
}

#[tokio::test]
async fn review_fix_check_async_memory_children_keep_and_release_root() {
    for mode in [StorageMode::Memory, StorageMode::Persistent] {
        let directory = tempfile::tempdir().unwrap();
        let (resources, entered, release) = review_gate_resources();
        let host = Host::open_persistent(directory.path(), settings("first"), resources.clone())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let mut request = create_request(mode);
        request.default_session = Some(SessionOptions {
            mode: Some(StorageMode::Memory),
            overrides: None,
        });
        request.initial_message = Some(input("active memory work"));
        host.create_agent_async(&connection.caller(), request)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        drop(connection);
        drop(host);
        assert!(
            Host::open_persistent(directory.path(), settings("second"), resources)
                .await
                .is_err(),
            "active memory child of {mode:?} Agent released root"
        );
        release.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(root) = DataRoot::acquire(directory.path()) {
                    drop(root);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn awaited_creation_retires_its_owner_before_immediate_root_reopen() {
    for (agent_mode, session_mode) in [
        (StorageMode::Memory, StorageMode::Memory),
        (StorageMode::Persistent, StorageMode::Memory),
        (StorageMode::Persistent, StorageMode::Persistent),
    ] {
        for _ in 0..24 {
            let directory = tempfile::tempdir().unwrap();
            let (resources, _) = resources();
            let host =
                Host::open_persistent(directory.path(), settings("first"), resources.clone())
                    .await
                    .unwrap();
            let connection = host.connect().unwrap();
            let mut request = create_request(agent_mode);
            request.default_session = Some(SessionOptions {
                mode: Some(session_mode),
                overrides: None,
            });
            let receipt = host
                .create_agent_async(&connection.caller(), request)
                .await
                .unwrap();
            receipt.validate_call_return().unwrap();
            assert!(receipt.result.steps.session.succeeded());
            drop(connection);
            drop(host);
            // No retry, sleep, or scheduling yield hides task retirement. No
            // Runtime was started, so only this creation could retain a lease.
            let reopened = Host::open_persistent(directory.path(), settings("second"), resources)
                .await
                .unwrap();
            let connection = reopened.connect().unwrap();
            assert_eq!(
                reopened.info(&connection.caller()).unwrap().host_id,
                receipt.host_id
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn awaited_metadata_retires_workers_before_immediate_root_reopen() {
    for _ in 0..24 {
        let directory = tempfile::tempdir().unwrap();
        let (host_id, agent_id, ids) = seed(directory.path());
        let (resources, _) = resources();
        let host = Host::open_persistent(directory.path(), settings("first"), resources.clone())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let agent = AgentRef {
            host_id,
            agent_id: agent_id.clone(),
        };
        let target = SessionRef {
            agent_id,
            session_id: ids[0].clone(),
        };
        let patch_agent = AgentConfigPatch {
            role: Some(Role {
                name: "confirmed Agent".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let patch_session = SessionConfigPatch {
            role: Some(Role {
                name: "confirmed Session".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let agent_result = host
            .update_agent_async(&caller, &agent, 7, &patch_agent)
            .await;
        let session_result = host
            .update_session_async(&caller, &target, 4, &patch_session)
            .await;
        assert_eq!(agent_result.unwrap().config_revision, 8);
        assert_eq!(session_result.unwrap().config_revision, 5);
        drop(connection);
        drop(host);
        let reopened = Host::open_persistent(directory.path(), settings("second"), resources)
            .await
            .unwrap();
        let connection = reopened.connect().unwrap();
        assert_eq!(
            reopened
                .agent_info(&connection.caller(), &agent)
                .unwrap()
                .config_revision,
            8
        );
        assert_eq!(
            reopened
                .session_info(&connection.caller(), &target)
                .unwrap()
                .config_revision,
            5
        );
    }
}

// Public completed Turn snapshots are execution facts, not a Host shutdown or
// task-join barrier. Before restart tests, observe the actual lease releasing;
// unexpected storage errors fail immediately, and no elapsed-time sleep is used.
async fn wait_for_root_release(path: &Path) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match DataRoot::acquire(path) {
                Ok(root) => {
                    drop(root);
                    return;
                }
                Err(StorageError::RootInUse) => tokio::task::yield_now().await,
                Err(error) => panic!("unexpected root acquisition failure: {error}"),
            }
        }
    })
    .await
    .expect("all old owner work must eventually release the root");
}

#[test]
fn review_meta_stop_wakes_after_late_child_metadata_save() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let (host_id, agent_id, mut ids) = seed(directory.path());
        ids.sort();
        let first = SessionRef {
            agent_id: agent_id.clone(),
            session_id: ids[0].clone(),
        };
        let last = SessionRef {
            agent_id: agent_id.clone(),
            session_id: ids[1].clone(),
        };
        let agent = AgentRef { host_id, agent_id };
        let (resources, _) = resources();
        let host = Host::open_persistent(directory.path(), settings("stop wake"), resources)
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();

        let (release_first, held_first) = std::sync::mpsc::channel::<()>();
        let (entered_first, entering_first) = tokio::sync::oneshot::channel();
        let first_blocker = tokio::task::spawn_blocking(move || {
            let _ = entered_first.send(());
            let _ = held_first.recv();
        });
        entering_first.await.unwrap();
        host.load_session(&caller, &last).unwrap();
        wait_state(&host, &caller, &last, |state| state.preparing).await;
        // Let the production load task enqueue its blocking read behind blocker1.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let (release_second, held_second) = std::sync::mpsc::channel::<()>();
        let (entered_second, entering_second) = tokio::sync::oneshot::channel();
        let second_blocker = tokio::task::spawn_blocking(move || {
            let _ = entered_second.send(());
            let _ = held_second.recv();
        });
        host.stop_agent(&caller, &agent).unwrap();
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().lifecycle,
            AgentLifecycle::Stopping
        );
        // Stop has walked the earlier, quiescent child and waits on the last child.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let task_host = Arc::clone(&host);
        let task_caller = caller.clone();
        let task_target = first.clone();
        let save = tokio::spawn(async move {
            task_host
                .update_session_async(
                    &task_caller,
                    &task_target,
                    4,
                    &SessionConfigPatch {
                        role: Some(Role {
                            name: "late accepted metadata".into(),
                            ..Role::default()
                        }),
                        ..Default::default()
                    },
                )
                .await
        });
        wait_state(&host, &caller, &first, |state| state.metadata_saving).await;
        release_first.send(()).unwrap();
        first_blocker.await.unwrap();
        entering_second.await.unwrap();
        wait_state(&host, &caller, &last, |state| !state.preparing).await;
        // Stop's second aggregate check now sees the first child's pending save.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().lifecycle,
            AgentLifecycle::Stopping
        );
        release_second.send(()).unwrap();
        second_blocker.await.unwrap();
        save.await.unwrap().unwrap();
        assert!(
            !host
                .session_state(&caller, &first)
                .unwrap()
                .data
                .metadata_saving
        );
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if host.agent_info(&caller, &agent).unwrap().lifecycle == AgentLifecycle::Stopped {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;

        assert!(
            stopped.is_ok(),
            "Agent stop lost the child's completion wake and stays stopping indefinitely"
        );
        host.start_agent(&caller, &agent).unwrap();
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().lifecycle,
            AgentLifecycle::Active
        );
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_meta_start_does_not_activate_unfinished_creation() {
    let directory = tempfile::tempdir().unwrap();
    let (inner, _) = resources();
    let gate = Arc::new(GatedValidationFixture {
        inner,
        count: std::sync::atomic::AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        released: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let host = Host::open_persistent(directory.path(), settings("creating fence"), gate.clone())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let request = create_request(StorageMode::Persistent);
    let operation = request.operation_id.clone();
    let task_host = Arc::clone(&host);
    let task_caller = caller.clone();
    let create =
        tokio::spawn(async move { task_host.create_agent_async(&task_caller, request).await });
    gate.entered.notified().await;
    let progress = host.receipt(&caller, &operation).unwrap();
    let agent = AgentRef {
        host_id: progress.host_id,
        agent_id: progress.agent_id.unwrap(),
    };
    assert_eq!(
        host.agent_info(&caller, &agent).unwrap().lifecycle,
        AgentLifecycle::Creating
    );
    let result = host.start_agent(&caller, &agent);
    let after_start = host.agent_info(&caller, &agent).unwrap().lifecycle;
    let updated = host
        .update_agent_async(
            &caller,
            &agent,
            0,
            &AgentConfigPatch {
                role: Some(Role {
                    name: "update before initial create completed".into(),
                    ..Role::default()
                }),
                ..Default::default()
            },
        )
        .await;
    gate.release();
    create.await.unwrap().unwrap();
    assert_eq!(after_start, AgentLifecycle::Creating);
    assert!(updated.is_err());
    assert!(
        result.is_err(),
        "start cannot publish Active while initial creation still owns unfinished work"
    );
}

#[test]
fn review_meta_check_close_and_stop_wait_for_owned_saves() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let (host_id, agent_id, ids) = seed(directory.path());
        let target = SessionRef {
            agent_id: agent_id.clone(),
            session_id: ids[0].clone(),
        };
        let agent = AgentRef { host_id, agent_id };
        let (resources, seen) = resources();
        let host = Host::open_persistent(directory.path(), settings("save close stop"), resources)
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, entering) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = held.recv();
        });
        entering.await.unwrap();
        let owner = Arc::clone(&host);
        let task_caller = caller.clone();
        let task_target = target.clone();
        let session_save = tokio::spawn(async move {
            owner
                .update_session_async(
                    &task_caller,
                    &task_target,
                    4,
                    &SessionConfigPatch {
                        role: Some(Role {
                            name: "confirmed Session value".into(),
                            ..Role::default()
                        }),
                        ..Default::default()
                    },
                )
                .await
        });
        let owner = Arc::clone(&host);
        let task_caller = caller.clone();
        let task_agent = agent.clone();
        let agent_save = tokio::spawn(async move {
            owner
                .update_agent_async(
                    &task_caller,
                    &task_agent,
                    7,
                    &AgentConfigPatch {
                        role: Some(Role {
                            name: "confirmed Agent value".into(),
                            ..Role::default()
                        }),
                        ..Default::default()
                    },
                )
                .await
        });
        wait_state(&host, &caller, &target, |state| state.metadata_saving).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !host.agent_info(&caller, &agent).unwrap().metadata_saving {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        host.close_session(&caller, &target).unwrap();
        host.stop_agent(&caller, &agent).unwrap();
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().lifecycle,
            AgentLifecycle::Stopping
        );
        assert_eq!(
            host.session_state(&caller, &target).unwrap().data.lifecycle,
            SessionLifecycle::Closing
        );
        assert_eq!(
            host.session_info(&caller, &target).unwrap().config_revision,
            4
        );
        assert_eq!(host.agent_info(&caller, &agent).unwrap().config_revision, 7);
        session_save.abort();
        agent_save.abort();
        assert!(session_save.await.unwrap_err().is_cancelled());
        assert!(agent_save.await.unwrap_err().is_cancelled());
        connection.close();
        let observer = host.connect().unwrap();
        let caller = observer.caller();
        release.send(()).unwrap();
        blocker.await.unwrap();
        wait_state(&host, &caller, &target, |state| {
            state.lifecycle == SessionLifecycle::Closed
        })
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while host.agent_info(&caller, &agent).unwrap().lifecycle != AgentLifecycle::Stopped {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let saved_agent = read_agent(&agent_path(directory.path(), &agent.agent_id));
        let saved_session = read_session(&session_path(directory.path(), &target));
        assert_eq!(saved_agent.data.config_revision, 8);
        assert_eq!(saved_agent.data.config.role.name, "confirmed Agent value");
        assert!(saved_agent.data.default_session_id.is_none());
        assert_eq!(saved_session.data.config_revision, 5);
        assert_eq!(
            saved_session.data.effective_config.role.name,
            "confirmed Session value"
        );
        assert!(saved_session.data.closed.is_some());
        assert!(seen.lock().unwrap().is_empty());
    });
}

#[test]
fn review_meta_check_pending_load_close_owns_root_until_closed() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let (_, agent_id, ids) = seed(directory.path());
        let target = SessionRef {
            agent_id,
            session_id: ids[0].clone(),
        };
        let (resources, seen) = resources();
        let host = Host::open_persistent(
            directory.path(),
            settings("load close root"),
            Arc::clone(&resources),
        )
        .await
        .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, entering) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = held.recv();
        });
        entering.await.unwrap();
        host.load_session(&caller, &target).unwrap();
        wait_state(&host, &caller, &target, |state| state.preparing).await;
        host.close_session(&caller, &target).unwrap();
        assert_eq!(
            host.session_state(&caller, &target).unwrap().data.lifecycle,
            SessionLifecycle::Closing
        );
        drop(connection);
        drop(host);
        assert!(DataRoot::acquire(directory.path()).is_err());
        release.send(()).unwrap();
        blocker.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(root) = DataRoot::acquire(directory.path()) {
                    drop(root);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            read_session(&session_path(directory.path(), &target))
                .data
                .closed
                .is_some()
        );
        assert!(
            read_agent(&agent_path(directory.path(), &target.agent_id))
                .data
                .default_session_id
                .is_none()
        );
        assert!(seen.lock().unwrap().is_empty());
        let reopened = Host::open_persistent(directory.path(), settings("reopen"), resources)
            .await
            .unwrap();
        let connection = reopened.connect().unwrap();
        assert_eq!(
            reopened
                .session_state(&connection.caller(), &target)
                .unwrap()
                .data
                .lifecycle,
            SessionLifecycle::Closed
        );
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_meta_check_confirmed_update_does_not_rebind_active_turn() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    let (resources, entered, release) = review_gate_resources();
    let host = Host::open_persistent(directory.path(), settings("turn binding"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    host.send(
        &caller,
        SendMessage::new(input("bound before save"), target.clone()),
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let first = host
        .turns(&caller, &target)
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(first.config_revision, 4);
    let updated = host
        .update_session_async(
            &caller,
            &target,
            4,
            &SessionConfigPatch {
                role: Some(Role {
                    name: "next turn config".into(),
                    ..Role::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.config_revision, 5);
    assert_eq!(
        host.turns(&caller, &target)
            .unwrap()
            .last()
            .unwrap()
            .config_revision,
        4
    );
    host.send(
        &caller,
        SendMessage::new(input("bound after save"), target.clone()),
    )
    .unwrap();
    release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let turns = host.turns(&caller, &target).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].config_revision, 4);
    assert_eq!(turns[1].config_revision, 5);
    assert_eq!(turns[0].persistence, PersistenceState::Persisted);
    release.add_permits(1);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        host.wait_turn(&caller, &target, &turns[1].id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Completed));
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert_eq!(
        read_session(&session_path(directory.path(), &target))
            .data
            .config_revision,
        5
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unrelated_failed_agent_record_does_not_block_nondefault_close() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("test"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[1].clone(),
    };
    let path = agent_path(directory.path(), &agent_id);
    std::fs::rename(path.join("agent.json"), path.join("original.json")).unwrap();
    std::fs::create_dir(path.join("agent.json")).unwrap();
    assert!(
        host.update_agent_async(
            &caller,
            &agent,
            7,
            &AgentConfigPatch {
                role: Some(Role {
                    name: "retained Agent candidate".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    std::fs::remove_dir(path.join("agent.json")).unwrap();
    std::fs::rename(path.join("original.json"), path.join("agent.json")).unwrap();
    host.close_session(&caller, &target).unwrap();
    let state = wait_state(&host, &caller, &target, |state| {
        state.lifecycle == SessionLifecycle::Closed || state.metadata_fault.is_some()
    })
    .await;
    assert_eq!(
        state.lifecycle,
        SessionLifecycle::Closed,
        "a retained unrelated Agent candidate cannot broaden a nondefault close dependency"
    );
    assert_eq!(
        read_agent(&path).data.default_session_id,
        Some(ids[0].clone())
    );
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_some()
    );
    assert!(
        host.agent_info(&caller, &agent)
            .unwrap()
            .metadata_fault
            .is_some()
    );
    assert_eq!(
        host.reconcile_agent_record(&caller, &agent)
            .await
            .unwrap()
            .config_revision,
        8
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pending_default_candidate_still_orders_its_target_close() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("test"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[1].clone(),
    };
    let path = agent_path(directory.path(), &agent_id);
    std::fs::rename(path.join("agent.json"), path.join("original.json")).unwrap();
    std::fs::create_dir(path.join("agent.json")).unwrap();
    assert!(
        host.set_default_session_async(&caller, &agent, Some(target.session_id.clone()))
            .await
            .is_err()
    );
    std::fs::remove_dir(path.join("agent.json")).unwrap();
    std::fs::rename(path.join("original.json"), path.join("agent.json")).unwrap();
    host.close_session(&caller, &target).unwrap();
    let state = wait_state(&host, &caller, &target, |state| {
        state.metadata_fault.is_some() || state.lifecycle == SessionLifecycle::Closed
    })
    .await;
    assert_eq!(state.lifecycle, SessionLifecycle::Closing);
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_none()
    );
    assert_eq!(
        host.agent_info(&caller, &agent).unwrap().default_session_id,
        Some(ids[0].clone())
    );
    assert_eq!(
        host.reconcile_agent_record(&caller, &agent)
            .await
            .unwrap()
            .default_session_id,
        Some(target.session_id.clone())
    );
    host.close_session(&caller, &target).unwrap();
    wait_state(&host, &caller, &target, |state| {
        state.lifecycle == SessionLifecycle::Closed
    })
    .await;
    assert!(read_agent(&path).data.default_session_id.is_none());
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_lifecycle_check_child_notify_leaves_no_owner_cycle() {
    // Exercise every Session construction route wired to the parent's token:
    // M/M, P/M, P/P, and persistent discovery.
    for case in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let seeded = (case == 3).then(|| seed(directory.path()));
        let (resources, entered, release) = review_gate_resources();
        let host = Host::open_persistent(directory.path(), settings("notify ownership"), resources)
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let (agent, target) = if let Some((host_id, agent_id, sessions)) = seeded {
            (
                AgentRef {
                    host_id,
                    agent_id: agent_id.clone(),
                },
                SessionRef {
                    agent_id,
                    session_id: sessions[0].clone(),
                },
            )
        } else {
            let mut request = create_request(if case == 0 {
                StorageMode::Memory
            } else {
                StorageMode::Persistent
            });
            request.default_session = Some(SessionOptions {
                mode: Some(if case == 2 {
                    StorageMode::Persistent
                } else {
                    StorageMode::Memory
                }),
                overrides: None,
            });
            let receipt = host.create_agent_async(&caller, request).await.unwrap();
            let agent_id = receipt.agent_id.unwrap();
            (
                AgentRef {
                    host_id: receipt.host_id,
                    agent_id: agent_id.clone(),
                },
                SessionRef {
                    agent_id,
                    session_id: receipt.session_id.unwrap(),
                },
            )
        };
        host.send(
            &caller,
            SendMessage::new(input("stop and release real work"), target.clone()),
        )
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        host.stop_agent(&caller, &agent).unwrap();
        release.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if host.agent_info(&caller, &agent).unwrap().lifecycle == AgentLifecycle::Stopped {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the child completion must settle the direct Agent's stop");
        assert_eq!(
            host.session_state(&caller, &target).unwrap().data.lifecycle,
            SessionLifecycle::Open
        );
        drop(connection);
        drop(host);
        wait_for_root_release(directory.path()).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_parent_child_metadata_updates_keep_confirmed_records() {
    for _ in 0..24 {
        let directory = tempfile::tempdir().unwrap();
        let (host_id, agent_id, ids) = seed(directory.path());
        let (resources, _) = resources();
        let host = Host::open_persistent(directory.path(), settings("first"), resources.clone())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let agent = AgentRef {
            host_id,
            agent_id: agent_id.clone(),
        };
        let target = SessionRef {
            agent_id,
            session_id: ids[0].clone(),
        };
        let patch_agent = AgentConfigPatch {
            role: Some(Role {
                name: "confirmed Agent".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let patch_session = SessionConfigPatch {
            role: Some(Role {
                name: "confirmed Session".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (agent_result, session_result) = tokio::join!(
            host.update_agent_async(&caller, &agent, 7, &patch_agent),
            host.update_session_async(&caller, &target, 4, &patch_session)
        );
        assert_eq!(agent_result.unwrap().config_revision, 8);
        assert_eq!(session_result.unwrap().config_revision, 5);
        drop(connection);
        drop(host);
        let reopened = Host::open_persistent(directory.path(), settings("second"), resources)
            .await
            .unwrap();
        let connection = reopened.connect().unwrap();
        assert_eq!(
            reopened
                .agent_info(&connection.caller(), &agent)
                .unwrap()
                .config_revision,
            8
        );
        assert_eq!(
            reopened
                .session_info(&connection.caller(), &target)
                .unwrap()
                .config_revision,
            5
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_close_unrelated_pending_default_does_not_block_third_child() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(
        directory.path(),
        settings("close unrelated default"),
        resources,
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    let created = host
        .create_session_async(
            &caller,
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: created.session_id.unwrap(),
    };
    let path = agent_path(directory.path(), &agent_id);
    std::fs::rename(path.join("agent.json"), path.join("saved-agent.json")).unwrap();
    std::fs::create_dir(path.join("agent.json")).unwrap();
    assert!(
        host.set_default_session_async(&caller, &agent, Some(ids[1].clone()))
            .await
            .is_err()
    );
    std::fs::remove_dir(path.join("agent.json")).unwrap();
    std::fs::rename(path.join("saved-agent.json"), path.join("agent.json")).unwrap();
    host.close_session(&caller, &target).unwrap();
    let state = wait_state(&host, &caller, &target, |state| {
        state.lifecycle == SessionLifecycle::Closed || state.metadata_fault.is_some()
    })
    .await;
    assert_eq!(state.lifecycle, SessionLifecycle::Closed);
    assert!(
        host.agent_info(&caller, &agent)
            .unwrap()
            .metadata_fault
            .is_some()
    );
    assert_eq!(
        read_agent(&path).data.default_session_id,
        Some(ids[0].clone())
    );
    let reconciled = host.reconcile_agent_record(&caller, &agent).await.unwrap();
    assert_eq!(reconciled.default_session_id, Some(ids[1].clone()));
    assert_eq!(
        read_agent(&path).data.default_session_id,
        Some(ids[1].clone())
    );
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_close_old_default_preserves_confirmed_new_default() {
    let directory = tempfile::tempdir().unwrap();
    let (host_id, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("close old default"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = AgentRef {
        host_id,
        agent_id: agent_id.clone(),
    };
    let target = SessionRef {
        agent_id: agent_id.clone(),
        session_id: ids[0].clone(),
    };
    let path = agent_path(directory.path(), &agent_id);
    std::fs::rename(path.join("agent.json"), path.join("saved-agent.json")).unwrap();
    std::fs::create_dir(path.join("agent.json")).unwrap();
    assert!(
        host.set_default_session_async(&caller, &agent, Some(ids[1].clone()))
            .await
            .is_err()
    );
    std::fs::remove_dir(path.join("agent.json")).unwrap();
    std::fs::rename(path.join("saved-agent.json"), path.join("agent.json")).unwrap();
    host.close_session(&caller, &target).unwrap();
    let state = wait_state(&host, &caller, &target, |state| {
        state.metadata_fault.is_some() || state.lifecycle == SessionLifecycle::Closed
    })
    .await;
    assert_eq!(state.lifecycle, SessionLifecycle::Closing);
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_none()
    );
    assert_eq!(
        read_agent(&path).data.default_session_id,
        Some(ids[0].clone())
    );
    assert_eq!(
        host.reconcile_agent_record(&caller, &agent)
            .await
            .unwrap()
            .default_session_id,
        Some(ids[1].clone())
    );
    host.close_session(&caller, &target).unwrap();
    wait_state(&host, &caller, &target, |state| {
        state.lifecycle == SessionLifecycle::Closed
    })
    .await;
    assert_eq!(
        read_agent(&path).data.default_session_id,
        Some(ids[1].clone()),
        "closing the former default must not clear the newly confirmed other target"
    );
    assert!(
        read_session(&session_path(directory.path(), &target))
            .data
            .closed
            .is_some()
    );
}

#[test]
fn canonical_loading_receipts_survive_connection_end_and_later_pause() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let (_, agent_id, ids) = seed(directory.path());
        let (resources, seen) = resources();
        let host = Host::open_persistent(directory.path(), settings("canonical load"), resources)
            .await
            .unwrap();
        let operator = host.connect().unwrap();
        let observer = host.connect().unwrap();
        let caller = operator.caller();
        let target = SessionRef {
            agent_id,
            session_id: ids[0].clone(),
        };
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, entering) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = held.recv();
        });
        entering.await.unwrap();
        host.pause(&caller, &target).unwrap();
        let load = host
            .session_load(
                &caller,
                &target,
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
            )
            .unwrap();
        let resume = host
            .session_resume(
                &caller,
                &target,
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
            )
            .unwrap();
        assert_eq!(load.result.residency, Residency::Loading);
        assert_eq!(resume.result.residency, Residency::Loading);
        assert_eq!(resume.result.scheduling, Scheduling::Paused);
        assert_eq!(resume.result.pause_reasons, [PauseReason::Explicit]);
        host.pause(&caller, &target).unwrap();
        operator.close();
        release.send(()).unwrap();
        blocker.await.unwrap();
        let state = wait_state(&host, &observer.caller(), &target, |state| !state.preparing).await;
        assert_eq!(state.residency, Residency::Loaded);
        assert_eq!(state.scheduling, Scheduling::Paused);
        assert!(seen.lock().unwrap().is_empty());
        for receipt in [load, resume] {
            assert_eq!(
                host.receipt_get(&observer.caller(), &receipt.operation_id)
                    .unwrap(),
                Receipt::SessionControl(receipt)
            );
        }
    });
}

#[tokio::test]
async fn canonical_creation_lookup_excludes_nested_steps_but_keeps_colliding_invocations() {
    let directory = tempfile::tempdir().unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(
        directory.path(),
        settings("canonical receipt lookup"),
        resources,
    )
    .await
    .unwrap();
    let operator = host.connect().unwrap();
    let caller = operator.caller();
    let request = create_request(StorageMode::Persistent);
    let operation = request.operation_id.clone();
    let created = host.create_agent_async(&caller, request).await.unwrap();
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::AgentCreate(created.clone())
    );
    let target = AgentRef {
        host_id: created.host_id,
        agent_id: created.agent_id.unwrap(),
    };
    let second = host
        .create_session_async(
            &caller,
            SessionCreate {
                operation_id: operation.clone(),
                target,
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(host.receipts(&caller, &operation).unwrap().len(), 2);
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap_err().kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );
    assert!(
        host.receipts(&caller, &operation)
            .unwrap()
            .contains(&Receipt::SessionCreate(second))
    );
}

#[tokio::test]
async fn canonical_close_keeps_formed_closing_fact_after_persistent_completion() {
    let directory = tempfile::tempdir().unwrap();
    let (_, agent_id, ids) = seed(directory.path());
    let (resources, _) = resources();
    let host = Host::open_persistent(directory.path(), settings("canonical close"), resources)
        .await
        .unwrap();
    let operator = host.connect().unwrap();
    let caller = operator.caller();
    let target = SessionRef {
        agent_id,
        session_id: ids[0].clone(),
    };
    let receipt = host
        .session_close(
            &caller,
            &target,
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(receipt.result.lifecycle, SessionCloseLifecycle::Closing);
    let state = wait_state(&host, &caller, &target, |state| {
        state.lifecycle == SessionLifecycle::Closed
    })
    .await;
    assert!(state.controller_revision > receipt.result.controller_revision);
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::SessionClose(receipt.clone())
    );
    let again = host
        .session_close(
            &caller,
            &target,
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(again.result.lifecycle, SessionCloseLifecycle::Closed);
    assert_eq!(
        host.receipt_get(&caller, &again.operation_id).unwrap(),
        Receipt::SessionClose(again)
    );
}

#[test]
fn canonical_resume_during_automatic_history_validation_returns_loading_then_continues() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let (_, agent_id, ids) = seed(directory.path());
        let (resources, seen) = resources();
        let host = Host::open_persistent(directory.path(), settings("automatic load"), resources)
            .await
            .unwrap();
        let operator = host.connect().unwrap();
        let caller = operator.caller();
        let target = SessionRef {
            agent_id,
            session_id: ids[0].clone(),
        };
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, entering) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = held.recv();
        });
        entering.await.unwrap();
        host.send(
            &caller,
            SendMessage::new(input("accepted before automatic load"), target.clone()),
        )
        .unwrap();
        wait_state(&host, &caller, &target, |state| {
            state.residency == Residency::Loading
        })
        .await;
        host.pause(&caller, &target).unwrap();
        let resume = host
            .session_resume(
                &caller,
                &target,
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
            )
            .unwrap();
        assert_eq!(resume.result.residency, Residency::Loading);
        assert_eq!(resume.result.scheduling, Scheduling::Paused);
        release.send(()).unwrap();
        blocker.await.unwrap();
        assert_eq!(
            completed(&host, &caller, &target).await.info.persistence,
            PersistenceState::Persisted
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(
            host.receipt_get(&caller, &resume.operation_id).unwrap(),
            Receipt::SessionControl(resume)
        );
    });
}
