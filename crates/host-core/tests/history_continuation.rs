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
    host.resume(&caller, &target)
        .expect("explicit resume must continue the retained original history save");
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        host.wait_turn(&caller, &target, &failed.id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
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
