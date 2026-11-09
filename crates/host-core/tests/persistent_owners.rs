//! Real temporary files, production owners and real Runtime. The model is an
//! explicitly scripted in-process fixture, not a live provider or TUI test.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_storage::*;
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
                Ok(ModelEvent::text_delta("fixture response").unwrap()),
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
    host.save_host(&HostRecord::new(host_id.clone(), settings("saved name"), 0))
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
    let history = std::fs::read_to_string(
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
        state.block_reason.unwrap().kind(),
        FaultKind::SessionHistoryLoadFailed
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
        FaultKind::AgentDefaultSessionUnavailable
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
