//! Independent acceptance checks: production Host/Journal/Runtime with a
//! deterministic in-process model fixture; no live provider or TUI coverage.
use futures_util::{StreamExt, stream};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_storage::{HistoryEntry, HistoryRecord};
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
}
impl Model for ModelFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.add_permits(1);
        let release = self.release.clone();
        Box::pin(async move {
            let tail = stream::once(async move {
                release.acquire_owned().await.unwrap().forget();
                stream::iter([
                    Ok(ModelEvent::text_delta("actual audited result").unwrap()),
                    Ok(ModelEvent::Finish {
                        reason: ModelFinishReason::Stop,
                    }),
                ])
            })
            .flatten();
            Ok(Box::pin(tail) as ModelStream)
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
async fn audit_explicit_resume_reconciles_original_unwritten_history_without_rerun() {
    let dir = tempfile::tempdir().unwrap();
    let (resources, entered, release, calls) = resources();
    let host = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    host.send(
        &caller,
        SendMessage::new(input("one original request"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    let path = dir
        .path()
        .join("agents")
        .join(target.agent_id.to_string())
        .join("sessions")
        .join(target.session_id.to_string())
        .join("history.jsonl");
    let saved_path = path.with_extension("audit-original");
    // Production Journal is already opened by load. Replace its path, not its
    // file bytes, so first append fails identity verification before writing.
    std::fs::rename(&path, &saved_path).unwrap();
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
    assert!(std::fs::read(&saved_path).unwrap().is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&saved_path, &path).unwrap();
    let resumed = host
        .session_resume(
            &caller,
            &target,
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .expect("explicit resume must continue the retained original history save");
    assert!(host.session_state(&caller, &target).unwrap().data.saving);
    let blocked = resumed
        .result
        .block_reason
        .as_ref()
        .expect("original save remains blocked until confirmed");
    assert_eq!(
        blocked.fault.as_ref().unwrap().kind(),
        Some(FaultKind::SessionHistoryWriteFailed)
    );
    assert!(!blocked.resolution.is_empty());
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        host.wait_turn(&caller, &target, &failed.id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert_eq!(
        host.receipt_get(&caller, &resumed.operation_id).unwrap(),
        Receipt::SessionControl(resumed)
    );
    assert_eq!(result.info.outcome, failed.outcome);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "continuation must never rerun model"
    );
    let records: Vec<HistoryRecord> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records.iter().map(|r| r.position).collect::<Vec<_>>(),
        (0..records.len() as u64).collect::<Vec<_>>()
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.entry, HistoryEntry::Input { .. }))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.entry, HistoryEntry::TurnFinished { .. }))
            .count(),
        1
    );
    let bytes = std::fs::read(&path).unwrap();
    host.resume(&caller, &target).unwrap();
    tokio::task::yield_now().await;
    assert_eq!(
        bytes,
        std::fs::read(&path).unwrap(),
        "stale resume must not duplicate records"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    host.unload(&caller, &target).unwrap();
    host.send(
        &caller,
        SendMessage::new(input("a genuinely new Turn"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    let next = host
        .turns(&caller, &target)
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_ne!(next.id, failed.id);
    assert_ne!(next.load_id, failed.load_id);
    assert_eq!(
        host.cancel_turn(&caller, &target, &failed.id, false)
            .unwrap()
            .status,
        CancelStatus::NotActive
    );
    host.resume(&caller, &target).unwrap();
    assert_eq!(
        host.turn_info(&caller, &target, &next.id).unwrap().state,
        TurnState::Running
    );
    release.add_permits(1);
    let next = tokio::time::timeout(
        Duration::from_secs(3),
        host.wait_turn(&caller, &target, &next.id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(next.info.outcome, Some(TurnOutcome::Completed));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(host.turns(&caller, &target).unwrap().len(), 2);
}

fn session_directory(root: &std::path::Path, target: &SessionRef) -> std::path::PathBuf {
    root.join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str())
}
#[tokio::test]
async fn resume_reconciles_only_this_sessions_original_metadata_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let (resources, _, _, calls) = resources();
    let host = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    let path = session_directory(dir.path(), &target).join("session.json");
    let original = host.session_info(&caller, &target).unwrap();
    let original_bytes = std::fs::read(&path).unwrap();
    std::fs::rename(&path, path.with_extension("original")).unwrap();
    std::fs::create_dir(&path).unwrap();
    let role = Role {
        name: "original candidate".into(),
        description: String::new(),
        instructions: "saved exact candidate".into(),
    };
    assert!(
        host.update_session_async(
            &caller,
            &target,
            original.config_revision,
            &SessionConfigPatch {
                role: Some(role.clone()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    host.pause(&caller, &target).unwrap();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("original"), &path).unwrap();
    let mut unrelated: minicore_host_storage::SessionRecord =
        serde_json::from_slice(&original_bytes).unwrap();
    unrelated.data.effective_config.role.name = "externally changed record".into();
    let unrelated_bytes = serde_json::to_vec(&unrelated).unwrap();
    std::fs::write(&path, &unrelated_bytes).unwrap();
    host.resume(&caller, &target).unwrap();
    until(|| {
        let state = host.session_state(&caller, &target).unwrap().data;
        (!state.metadata_saving && state.metadata_fault.is_some()).then_some(())
    })
    .await;
    assert_eq!(std::fs::read(&path).unwrap(), unrelated_bytes);
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config,
        original.config
    );
    assert_eq!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .scheduling,
        Scheduling::Paused
    );
    std::fs::write(&path, original_bytes).unwrap();
    host.resume(&caller, &target).unwrap();
    until(|| {
        let state = host.session_state(&caller, &target).unwrap().data;
        (!state.metadata_saving && state.metadata_fault.is_none()).then_some(())
    })
    .await;
    let saved = host.session_info(&caller, &target).unwrap();
    assert_eq!(saved.config_revision, original.config_revision + 1);
    assert_eq!(saved.config.role, role);
    assert_eq!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .scheduling,
        Scheduling::Auto
    );
    let bytes = std::fs::read(&path).unwrap();
    host.resume(&caller, &target).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn resume_settles_history_and_metadata_originals_without_cross_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let (resources, entered, release, calls) = resources();
    let host = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    let directory = session_directory(dir.path(), &target);
    host.send(
        &caller,
        SendMessage::new(input("one original execution"), target.clone()),
    )
    .unwrap();
    entered.acquire().await.unwrap().forget();
    let history = directory.join("history.jsonl");
    std::fs::rename(&history, history.with_extension("original")).unwrap();
    std::fs::write(&history, b"").unwrap();
    release.add_permits(1);
    let failed = until(|| {
        host.turns(&caller, &target)
            .unwrap()
            .last()
            .filter(|turn| {
                turn.state == TurnState::Finalizing && turn.persistence == PersistenceState::Failed
            })
            .cloned()
    })
    .await;
    let metadata = directory.join("session.json");
    let original = host.session_info(&caller, &target).unwrap();
    std::fs::rename(&metadata, metadata.with_extension("original")).unwrap();
    std::fs::create_dir(&metadata).unwrap();
    let role = Role {
        name: "same Session candidate".into(),
        description: String::new(),
        instructions: "new config only after confirmation".into(),
    };
    assert!(
        host.update_session_async(
            &caller,
            &target,
            original.config_revision,
            &SessionConfigPatch {
                role: Some(role.clone()),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    std::fs::remove_file(&history).unwrap();
    std::fs::rename(history.with_extension("original"), &history).unwrap();
    host.send(
        &caller,
        SendMessage::new(input("a separately queued follow-up"), target.clone()),
    )
    .unwrap();
    // The first explicit request confirms history, while still-broken metadata
    // remains its own original fault/version. Neither success stands in for both.
    host.resume(&caller, &target).unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        host.wait_turn(&caller, &target, &failed.id),
    )
    .await
    .unwrap()
    .unwrap();
    until(|| {
        let state = host.session_state(&caller, &target).unwrap().data;
        (!state.metadata_saving && state.metadata_fault.is_some()).then_some(())
    })
    .await;
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert_eq!(result.info.outcome, failed.outcome);
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config,
        original.config
    );
    assert_eq!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .next_turn
            .len(),
        1
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "failed companion metadata must not resume queued business"
    );
    let confirmed_history = std::fs::read(&history).unwrap();
    std::fs::remove_dir(&metadata).unwrap();
    std::fs::rename(metadata.with_extension("original"), &metadata).unwrap();
    host.resume(&caller, &target).unwrap();
    until(|| {
        let state = host.session_state(&caller, &target).unwrap().data;
        (!state.metadata_saving && state.metadata_fault.is_none()).then_some(())
    })
    .await;
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config.role,
        role
    );
    assert_eq!(std::fs::read(&history).unwrap(), confirmed_history);
    entered.acquire().await.unwrap().forget();
    let later = host
        .turns(&caller, &target)
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_ne!(later.id, failed.id);
    assert_eq!(later.config_revision, original.config_revision + 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    release.add_permits(1);
    host.wait_turn(&caller, &target, &later.id).await.unwrap();
    assert_eq!(host.turns(&caller, &target).unwrap().len(), 2);
}

#[tokio::test]
async fn resume_can_finish_original_closure_record_without_reopening_session() {
    let dir = tempfile::tempdir().unwrap();
    let (resources, _, _, calls) = resources();
    let host = Host::open_persistent(dir.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    let path = session_directory(dir.path(), &target).join("session.json");
    std::fs::rename(&path, path.with_extension("original")).unwrap();
    std::fs::create_dir(&path).unwrap();
    host.close_session(&caller, &target).unwrap();
    until(|| {
        let state = host.session_state(&caller, &target).unwrap().data;
        (state.lifecycle == SessionLifecycle::Closing
            && !state.metadata_saving
            && state.metadata_fault.is_some())
        .then_some(())
    })
    .await;
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("original"), &path).unwrap();
    host.resume(&caller, &target).unwrap();
    until(|| {
        (host.session_state(&caller, &target).unwrap().data.lifecycle == SessionLifecycle::Closed)
            .then_some(())
    })
    .await;
    assert_eq!(
        host.resume(&caller, &target).unwrap_err().kind(),
        Some(FaultKind::SessionLifecycleClosed)
    );
    let saved: minicore_host_storage::SessionRecord =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(saved.data.closed.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
