//! Real temporary files, production owners and real Runtime. The model is an
//! explicitly scripted in-process fixture, not a live provider or TUI test.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use minicore_host_storage::testing::read_history_bytes;
use minicore_host_storage::*;
use std::{
    collections::BTreeSet,
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
                options: ExecutionOptions::default(),
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
async fn fixture() -> (tempfile::TempDir, Arc<Host>, ConnectionHandle) {
    let dir = tempfile::tempdir().unwrap();
    let (resources, _) = resources();
    let host = Host::open_persistent(dir.path(), settings("deletion fixture"), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    (dir, host, connection)
}
async fn create(
    host: &Arc<Host>,
    caller: &CallerContext,
    mode: StorageMode,
) -> (AgentRef, SessionRef, AgentCreateReceipt) {
    let receipt = host
        .create_agent_async(
            caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "owned deletion fixture".into(),
                mode,
                config: config(),
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let agent = AgentRef {
        host_id: receipt.host_id.clone(),
        agent_id: receipt.agent_id.clone().unwrap(),
    };
    let session = SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: receipt.session_id.clone().unwrap(),
    };
    (agent, session, receipt)
}
async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while !ready() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn path(dir: &std::path::Path, target: &SessionRef) -> std::path::PathBuf {
    dir.join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str())
}
fn delete(close: bool) -> SessionDeleteRequest {
    SessionDeleteRequest {
        operation_id: OperationId::new().unwrap(),
        close,
    }
}
fn cascade(ids: Vec<SessionId>, close: bool) -> AgentDeleteRequest {
    AgentDeleteRequest {
        operation_id: OperationId::new().unwrap(),
        scope: AgentDeleteScope::Sessions {
            session_ids: ids,
            close,
        },
    }
}
async fn deleted(host: &Host, caller: &CallerContext, target: &SessionRef) {
    until(|| matches!(host.session_info(caller,target),Err(error) if error.kind()==Some(FaultKind::AgentSessionNotFound))).await;
}
async fn closed(host: &Host, caller: &CallerContext, target: &SessionRef) {
    host.session_close(
        caller,
        target,
        SessionCloseRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    until(|| {
        host.session_state(caller, target).unwrap().data.lifecycle == SessionLifecycle::Closed
    })
    .await;
    // The Session can publish closed before its real Agent close worker exits.
    tokio::task::yield_now().await;
}
#[tokio::test]
async fn defaults_reject_open_sessions_active_agents_and_retained_children() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (agent, session, _) = create(&host, &caller, StorageMode::Persistent).await;
    assert_eq!(
        host.session_delete(&caller, &session, delete(false))
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionDeleteInvalid)
    );
    let empty = || AgentDeleteRequest {
        operation_id: OperationId::new().unwrap(),
        scope: AgentDeleteScope::Empty {},
    };
    assert_eq!(
        host.agent_delete(&caller, &agent, empty())
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentDeleteInvalid)
    );
    host.agent_stop(
        &caller,
        &agent,
        AgentStopRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    until(|| host.agent_info(&caller, &agent).unwrap().lifecycle == AgentLifecycle::Stopped).await;
    assert_eq!(
        host.agent_delete(&caller, &agent, empty())
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentDeleteInvalid)
    );
    assert!(path(dir.path(), &session).join("session.json").exists());
    host.session_delete(&caller, &session, delete(true))
        .unwrap();
    deleted(&host, &caller, &session).await;
    let receipt = host.agent_delete(&caller, &agent, empty()).unwrap();
    until(|| host.agent_info(&caller, &agent).is_err()).await;
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::AgentDelete(receipt)
    );
    assert!(
        !dir.path()
            .join("agents")
            .join(agent.agent_id.as_str())
            .exists()
    );
}
#[tokio::test]
async fn default_closed_deletion_preserves_another_session_and_immutable_parent_receipts() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (agent, session, creation) = create(&host, &caller, StorageMode::Persistent).await;
    let next = host
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
    let other = SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: next.session_id.unwrap(),
    };
    let old_close = SessionCloseRequest {
        operation_id: OperationId::new().unwrap(),
    };
    let original = host.session_close(&caller, &session, old_close).unwrap();
    until(|| {
        host.session_state(&caller, &session)
            .unwrap()
            .data
            .lifecycle
            == SessionLifecycle::Closed
    })
    .await;
    let request = delete(false);
    let operation = request.operation_id.clone();
    // The close worker and deletion share the Agent admission boundary.
    let receipt = loop {
        match host.session_delete(&caller, &session, request.clone()) {
            Ok(r) => break r,
            Err(e) if e.kind() == Some(FaultKind::SessionBusy) => tokio::task::yield_now().await,
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(receipt.result.lifecycle, SessionCloseLifecycle::Closed);
    deleted(&host, &caller, &session).await;
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::SessionDelete(receipt)
    );
    assert_eq!(
        host.receipt_get(&caller, &original.operation_id)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptNotFound)
    );
    assert_eq!(
        host.receipt_get(&caller, &creation.operation_id).unwrap(),
        Receipt::AgentCreate(creation)
    );
    assert!(host.session_info(&caller, &other).is_ok());
    assert!(!path(dir.path(), &session).exists());
    assert!(path(dir.path(), &other).exists());
    let record: AgentRecord = serde_json::from_slice(
        &std::fs::read(
            dir.path()
                .join("agents")
                .join(agent.agent_id.as_str())
                .join("agent.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record.data.default_session_id, None);
    assert_eq!(
        host.agent_info(&caller, &agent).unwrap().default_session_id,
        None
    );
}
#[tokio::test]
async fn explicit_cascade_matches_direct_owner_and_never_deletes_independent_artifacts() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (agent, session, _) = create(&host, &caller, StorageMode::Persistent).await;
    let (other_agent, other, _) = create(&host, &caller, StorageMode::Persistent).await;
    let artifact = dir.path().join("independent-workspace-output");
    std::fs::write(&artifact, b"keep me").unwrap();
    for ids in [
        vec![other.session_id.clone()],
        vec![session.session_id.clone(), other.session_id.clone()],
    ] {
        assert_eq!(
            host.agent_delete(&caller, &agent, cascade(ids, true))
                .unwrap_err()
                .kind(),
            Some(FaultKind::AgentDeleteInvalid)
        );
    }
    assert_eq!(
        host.session_state(&caller, &session)
            .unwrap()
            .data
            .lifecycle,
        SessionLifecycle::Open
    );
    let receipt = host
        .agent_delete(
            &caller,
            &agent,
            cascade(vec![session.session_id.clone()], true),
        )
        .unwrap();
    assert_eq!(receipt.result.lifecycle, AgentDeleteLifecycle::Retiring);
    until(|| host.agent_info(&caller, &agent).is_err()).await;
    assert_eq!(std::fs::read(&artifact).unwrap(), b"keep me");
    assert!(path(dir.path(), &other).exists());
    assert!(host.agent_info(&caller, &other_agent).is_ok());
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::AgentDelete(receipt)
    );
}
#[tokio::test]
async fn unknown_nested_artifact_blocks_removal_and_reports_real_partial_cascade() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (agent, first, _) = create(&host, &caller, StorageMode::Persistent).await;
    let second = host
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
    let second = SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: second.session_id.unwrap(),
    };
    let mut sessions = [first, second];
    sessions.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    let artifact = path(dir.path(), &sessions[1]).join("unowned-workspace-file");
    std::fs::write(&artifact, b"untouched").unwrap();
    let request = cascade(
        sessions.iter().map(|s| s.session_id.clone()).collect(),
        true,
    );
    let receipt = host.agent_delete(&caller, &agent, request).unwrap();
    until(|| {
        host.agent_info(&caller, &agent)
            .unwrap()
            .deletion
            .as_ref()
            .is_some_and(|d| matches!(d.data, DataDeletion::Failed { .. }))
    })
    .await;
    let progress = host.agent_info(&caller, &agent).unwrap().deletion.unwrap();
    assert_eq!(progress.sessions[0].result.data, DataDeletion::Deleted {});
    assert!(matches!(
        progress.sessions[1].result.data,
        DataDeletion::Failed { .. }
    ));
    assert!(!path(dir.path(), &sessions[0]).exists());
    assert_eq!(std::fs::read(&artifact).unwrap(), b"untouched");
    assert!(host.session_info(&caller, &sessions[1]).is_ok());
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::AgentDelete(receipt)
    );
    assert_eq!(
        host.agent_start(
            &caller,
            &agent,
            AgentStartRequest {
                operation_id: OperationId::new().unwrap()
            }
        )
        .unwrap_err()
        .kind(),
        Some(FaultKind::AgentLifecycleUnavailable)
    );
}
#[tokio::test]
async fn reused_operation_is_correlation_and_missing_objects_are_not_success() {
    let (_dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (_, first, _) = create(&host, &caller, StorageMode::Persistent).await;
    let (_, second, _) = create(&host, &caller, StorageMode::Persistent).await;
    let request = delete(true);
    host.session_delete(&caller, &first, request.clone())
        .unwrap();
    host.session_delete(&caller, &second, request.clone())
        .unwrap();
    assert_eq!(
        host.receipt_get(&caller, &request.operation_id)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );
    deleted(&host, &caller, &first).await;
    deleted(&host, &caller, &second).await;
    assert_eq!(
        host.session_delete(&caller, &first, request.clone())
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    assert_eq!(
        host.receipts(&caller, &request.operation_id).unwrap().len(),
        2
    );
}
#[tokio::test]
async fn memory_removal_is_reclamation_and_cannot_be_deleted_again() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (agent, _, _) = create(&host, &caller, StorageMode::Persistent).await;
    let child = host
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
        session_id: child.session_id.unwrap(),
    };
    assert!(!path(dir.path(), &target).exists());
    let receipt = host.session_delete(&caller, &target, delete(true)).unwrap();
    deleted(&host, &caller, &target).await;
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::SessionDelete(receipt)
    );
    assert_eq!(
        host.session_delete(&caller, &target, delete(true))
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    let (memory, target, _) = create(&host, &caller, StorageMode::Memory).await;
    let receipt = host
        .agent_delete(&caller, &memory, cascade(vec![target.session_id], true))
        .unwrap();
    until(|| host.agent_info(&caller, &memory).is_err()).await;
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::AgentDelete(receipt)
    );
}
#[tokio::test]
async fn ordinary_close_and_stop_keep_files_until_explicit_deletion() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (agent, target, _) = create(&host, &caller, StorageMode::Persistent).await;
    host.send(
        &caller,
        SendMessage::new(input("ordinary retained history"), target.clone()),
    )
    .unwrap();
    until(|| !host.turns(&caller, &target).unwrap().is_empty()).await;
    let turn = host.turns(&caller, &target).unwrap()[0].id.clone();
    host.wait_turn(&caller, &target, &turn).await.unwrap();
    closed(&host, &caller, &target).await;
    host.agent_stop(
        &caller,
        &agent,
        AgentStopRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    until(|| host.agent_info(&caller, &agent).unwrap().lifecycle == AgentLifecycle::Stopped).await;
    // Business close/stop leaves the persistent controller's journal lease alive.
    // The stopped Agent is quiescent, so inspect the actual bytes without retiring it.
    let history = read_history_bytes(path(dir.path(), &target).join("history.jsonl")).unwrap();
    assert!(
        String::from_utf8(history)
            .unwrap()
            .contains("ordinary retained history")
    );
    assert!(path(dir.path(), &target).join("session.json").exists());
}

#[tokio::test]
async fn failed_delete_retains_real_error_and_releases_locator_at_controlled_shutdown() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (_, target, _) = create(&host, &caller, StorageMode::Persistent).await;
    let artifact = path(dir.path(), &target).join("separate-artifact");
    std::fs::write(&artifact, b"keep").unwrap();
    let receipt = host.session_delete(&caller, &target, delete(true)).unwrap();
    until(|| {
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .deletion
            .is_some_and(|d| matches!(d.data, DataDeletion::Failed { .. }))
    })
    .await;
    assert_eq!(
        host.session_state(&caller, &target).unwrap().data.lifecycle,
        SessionLifecycle::Closed
    );
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::SessionDelete(receipt)
    );
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    until(|| host.info(&caller).unwrap().lifecycle == HostLifecycle::Stopped).await;
    assert_eq!(std::fs::read(&artifact).unwrap(), b"keep");
    let (resources, _) = resources();
    let reopened = Host::open_persistent(
        dir.path(),
        settings("reopen after failed delete"),
        resources,
    )
    .await
    .unwrap();
    let next = reopened.connect().unwrap();
    assert_eq!(
        reopened
            .session_state(&next.caller(), &target)
            .unwrap()
            .data
            .lifecycle,
        SessionLifecycle::Closed
    );
}

struct PendingPreparation {
    inner: Arc<dyn ResourceResolver>,
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}
impl ResourceResolver for PendingPreparation {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.inner.resolve(config).await
        })
    }
}
#[tokio::test]
async fn deletion_cancels_unfinished_preparation_without_starting_runtime_or_reopening_input() {
    let dir = tempfile::tempdir().unwrap();
    let (inner, seen) = resources();
    let resources = Arc::new(PendingPreparation {
        inner,
        entered: tokio::sync::Semaphore::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let host = Host::open_persistent(
        dir.path(),
        settings("pending preparation"),
        resources.clone(),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let (_, target, _) = create(&host, &caller, StorageMode::Persistent).await;
    host.send(
        &caller,
        SendMessage::new(input("accepted before delete"), target.clone()),
    )
    .unwrap();
    resources.entered.acquire().await.unwrap().forget();
    assert!(host.session_state(&caller, &target).unwrap().data.preparing);
    host.session_delete(&caller, &target, delete(true)).unwrap();
    deleted(&host, &caller, &target).await;
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(resources.release.available_permits(), 0);
    assert!(!path(dir.path(), &target).exists());
}

#[tokio::test]
async fn accepted_metadata_saves_settle_before_delete_and_agent_admission_rejects_own_save() {
    let (dir, host, connection) = fixture().await;
    let caller = connection.caller();
    let (agent, target, _) = create(&host, &caller, StorageMode::Persistent).await;
    let default = host
        .agent_set_default_session(
            &caller,
            &agent,
            AgentSetDefaultSessionRequest {
                operation_id: OperationId::new().unwrap(),
                session_id: None,
            },
        )
        .unwrap();
    assert!(matches!(
        default.result,
        AgentSetDefaultSessionResult::Saving { .. }
    ));
    assert_eq!(
        host.agent_delete(
            &caller,
            &agent,
            cascade(vec![target.session_id.clone()], true)
        )
        .unwrap_err()
        .kind(),
        Some(FaultKind::AgentDeleteInvalid)
    );
    until(|| !host.agent_info(&caller, &agent).unwrap().metadata_saving).await;
    let artifact = path(dir.path(), &target).join("independent");
    std::fs::write(&artifact, b"keep").unwrap();
    let update = host
        .session_update(
            &caller,
            &target,
            SessionUpdateRequest {
                operation_id: OperationId::new().unwrap(),
                expected_config_revision: 0,
                changes: SessionUpdateChanges {
                    title: NullablePatch::Set("accepted before close".into()),
                    config: Default::default(),
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(update.result, MetadataUpdateResult::Saving { .. }));
    let receipt = host.session_delete(&caller, &target, delete(true)).unwrap();
    until(|| {
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .deletion
            .is_some_and(|d| matches!(d.data, DataDeletion::Failed { .. }))
    })
    .await;
    let saved: SessionRecord = serde_json::from_slice(
        &std::fs::read(path(dir.path(), &target).join("session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.data.title.as_deref(), Some("accepted before close"));
    assert!(saved.data.closed.is_some());
    assert_eq!(saved.data.config_revision, 1);
    assert_eq!(std::fs::read(&artifact).unwrap(), b"keep");
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::SessionDelete(receipt)
    );
    assert_eq!(
        host.receipt_get(&caller, &update.operation_id).unwrap(),
        Receipt::SessionUpdate(update)
    );
}
