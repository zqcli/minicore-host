//! Real Session owners, Runtime and journal. Only resource faults/model output
//! are injected fixtures, so attempt counts can prove there is no retry loop.
use super::*;

const RESOURCE_FAULTS: [FaultKind; 3] = [
    FaultKind::SessionConfigInvalid,
    FaultKind::SessionConfigConflict,
    FaultKind::SessionResourceMissing,
];

struct FailingResources {
    resources: Resources,
    failure: Mutex<Option<FaultKind>>,
    attempts: AtomicUsize,
}
impl ResourceResolver for FailingResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.resources.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(kind) = *self.failure.lock().unwrap() {
                return Err(Fault::new(kind, format!("fixture preparation {attempt}")));
            }
            self.resources.resolve(config).await
        })
    }
}

struct RecoveryFixture {
    _directory: tempfile::TempDir,
    session: Arc<SessionController>,
    connection: crate::ConnectionHandle,
    resources: Arc<FailingResources>,
    model: Arc<FixtureModel>,
    store: Option<SessionStore>,
}
impl RecoveryFixture {
    fn new(mode: StorageMode, failure: FaultKind) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let host_id = HostId::new().unwrap();
        let incarnation = HostIncarnation::new().unwrap();
        let agent_id = AgentId::new().unwrap();
        let id = SessionId::new().unwrap();
        let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
        let connection = access.connect().unwrap();
        let model = model(vec![final_events("repaired")]);
        let resources = Arc::new(FailingResources {
            resources: Resources {
                model: model.clone(),
                tools: ToolSet::default(),
            },
            failure: Mutex::new(Some(failure)),
            attempts: AtomicUsize::new(0),
        });
        let config = SessionConfig {
            instructions: String::new(),
            model: "test/model".parse().unwrap(),
            ..Default::default()
        };
        let store = (mode == StorageMode::Persistent).then(|| {
            let root = DataRoot::acquire(directory.path()).unwrap();
            let host = root.host(host_id.clone());
            host.save_host(&HostRecord::new(host_id.clone(), (), 0))
                .unwrap();
            let agent = host.agent(agent_id.clone(), mode);
            agent
                .save(&AgentRecord::new(
                    agent_id.clone(),
                    host_id.clone(),
                    AgentData {
                        configuration_source: None,
                        name: "resource recovery fixture".into(),
                        mode,
                        config: AgentConfig::default(),
                        config_revision: 0,
                        default_session_id: None,
                    },
                ))
                .unwrap();
            agent.session(id.clone(), mode).unwrap()
        });
        let enabled = Arc::new(AtomicBool::new(true));
        let session = if let Some(store) = &store {
            let record = SessionRecord::new(
                id,
                agent_id,
                SessionData {
                    created_at: minicore_host_protocol::Timestamp::now(),
                    last_activity_at: minicore_host_protocol::Timestamp::now(),
                    title: None,
                    mode,
                    agent_config_revision: 0,
                    config_revision: 0,
                    effective_config: config,
                    parent: None,
                    closed: None,
                },
            );
            store.save(&record).unwrap();
            SessionController::from_record(
                record,
                store.clone(),
                host_id,
                incarnation,
                access,
                resources.clone(),
                enabled,
                false,
            )
        } else {
            SessionController::new(
                id,
                agent_id,
                host_id,
                incarnation,
                config,
                access,
                resources.clone(),
                Arc::new(MemoryHistory),
                Vec::new(),
                enabled,
            )
        };
        Self {
            _directory: directory,
            session,
            connection,
            resources,
            model,
            store,
        }
    }
    fn send(&self) -> MessageId {
        let message_id = MessageId::new().unwrap();
        self.session
            .send(
                &self.connection.caller(),
                MessageInput {
                    message_id: message_id.clone(),
                    mode: MessageMode::FollowUp,
                    text: "accepted once".into(),
                    in_reply_to: None,
                },
            )
            .unwrap();
        message_id
    }
    async fn settled(&self) {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.session.wait_quiescent_owned(),
        )
        .await
        .expect("Session work did not settle");
    }
    fn assert_queued(&self, id: &MessageId, attempts: usize) {
        let state = self.session.state.lock().unwrap();
        assert!(state.active.is_none());
        assert!(state.completed.is_empty());
        assert_eq!(state.next_turn.len(), 1);
        assert_eq!(&state.next_turn[0].message.message_id, id);
        assert!(state.history.is_empty());
        assert_eq!(self.resources.attempts.load(Ordering::SeqCst), attempts);
        assert_eq!(self.model.calls.load(Ordering::SeqCst), 0);
    }
    fn assert_completed(&self, id: &MessageId, attempts: usize) {
        let state = self.session.state.lock().unwrap();
        assert!(state.block_reason.is_none());
        assert!(state.next_turn.is_empty());
        assert_eq!(state.completed.len(), 1);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(&state.messages[0].input.message_id, id);
        assert!(matches!(
            state.messages[0].binding,
            MessageBinding::Bound { .. }
        ));
        assert_eq!(
            state.completed.values().next().unwrap().info.outcome,
            Some(TurnOutcome::Completed)
        );
        assert_eq!(self.resources.attempts.load(Ordering::SeqCst), attempts);
        assert_eq!(self.model.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn synchronous_memory_metadata_does_not_need_a_current_or_retained_executor() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let fixture = RecoveryFixture::new(StorageMode::Memory, FaultKind::SessionResourceMissing);
    let caller = fixture.connection.caller();
    assert!(fixture.session.executor().is_none());
    let patch = SessionConfigPatch {
        role: Some(Role {
            instructions: "updated without an executor".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let updated = fixture.session.update(&caller, 0, &patch).unwrap();
    assert_eq!(updated.config_revision, 1);
    assert_eq!(updated.config.role, patch.role.unwrap());
    assert!(fixture.session.executor().is_none());
    assert_eq!(fixture.resources.attempts.load(Ordering::SeqCst), 0);

    let agent = crate::agent::AgentOwner::new(
        fixture.session.agent_id.clone(),
        fixture.session.host_id.clone(),
        fixture.session.incarnation.clone(),
        "synchronous metadata fixture".into(),
        AgentConfig::default(),
        fixture.session.access.clone(),
        fixture.resources.clone(),
    );
    let patch = AgentConfigPatch {
        role: Some(updated.config.role),
        ..Default::default()
    };
    let updated = agent.update(&caller, 0, &patch).unwrap();
    assert_eq!(updated.config_revision, 1);
    assert_eq!(updated.config.role, patch.role.unwrap());
    agent.set_default(&caller, None).unwrap();
    assert!(agent.info(&caller).unwrap().default_session_id.is_none());
}

#[test]
fn synchronous_persistent_update_keeps_authority_and_lifecycle_refusals() {
    let fixture = RecoveryFixture::new(StorageMode::Persistent, FaultKind::SessionResourceMissing);
    let observer = fixture.session.access.connect().unwrap();
    let patch = SessionConfigPatch::default();
    assert_eq!(
        fixture
            .session
            .update(&observer.caller(), 0, &patch)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly),
    );
    let caller = fixture.connection.caller();
    assert_eq!(
        fixture
            .session
            .update(&caller, 0, &patch)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostMethodUnavailable),
    );
    fixture.session.state.lock().unwrap().lifecycle = SessionLifecycle::Closed;
    assert_eq!(
        fixture
            .session
            .update(&caller, 0, &patch)
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionLifecycleClosed),
    );
    assert!(fixture.session.executor().is_none());
}

async fn resume_reprepares(mode: StorageMode) {
    for kind in RESOURCE_FAULTS {
        let fixture = RecoveryFixture::new(mode, kind);
        let id = fixture.send();
        fixture.settled().await;
        fixture.assert_queued(&id, 1);
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        fixture.settled().await;
        fixture.assert_queued(&id, 2);
        let fault = fixture
            .session
            .state
            .lock()
            .unwrap()
            .block_reason
            .clone()
            .unwrap();
        assert_eq!(fault.kind(), Some(kind));
        assert_eq!(fault.message(), "fixture preparation 2");
        // Repairing a dependency is not an implicit retry. The next explicit
        // resume owns exactly one new preparation of the original input.
        *fixture.resources.failure.lock().unwrap() = None;
        fixture.assert_queued(&id, 2);
        fixture
            .session
            .resume(&fixture.connection.caller())
            .unwrap();
        fixture.settled().await;
        fixture.assert_completed(&id, 3);
        assert_eq!(
            fixture
                .session
                .info(&fixture.connection.caller())
                .unwrap()
                .config_revision,
            0
        );
    }
}

async fn update_releases_only_resource_block(mode: StorageMode, candidate: bool) {
    for kind in RESOURCE_FAULTS {
        let fixture = RecoveryFixture::new(mode, kind);
        let caller = fixture.connection.caller();
        let id = fixture.send();
        fixture.settled().await;
        fixture.assert_queued(&id, 1);
        fixture.session.pause(&caller).unwrap();
        *fixture.resources.failure.lock().unwrap() = None;
        let original = fixture.session.info(&caller).unwrap();
        // A rejected stale update never clears the startup fault.
        assert_eq!(
            fixture
                .session
                .update_async(&caller, 9, &SessionConfigPatch::default())
                .await
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionConfigConflict)
        );
        assert_eq!(
            fixture
                .session
                .state
                .lock()
                .unwrap()
                .block_reason
                .as_ref()
                .unwrap()
                .kind(),
            Some(kind)
        );
        let mut config = original.config.clone();
        config.role.instructions = "repaired role".into();
        let updated = if candidate {
            fixture
                .session
                .update_candidate_async(&caller, 0, config.clone(), None)
                .await
                .unwrap()
        } else {
            fixture
                .session
                .update_async(
                    &caller,
                    0,
                    &SessionConfigPatch {
                        role: Some(config.role.clone()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
        };
        assert_eq!(updated.config_revision, 1);
        assert_eq!(updated.config, config);
        {
            let state = fixture.session.state.lock().unwrap();
            assert!(
                state.block_reason.is_none(),
                "successful {mode:?} update kept {kind:?}"
            );
            assert!(
                state.paused,
                "configuration update must not undo explicit pause"
            );
        }
        fixture.assert_queued(&id, 1);
        if let Some(store) = &fixture.store {
            let record = store.load().unwrap();
            assert_eq!(record.data.config_revision, 1);
            assert_eq!(record.data.effective_config, updated.config);
        }
        fixture.session.resume(&caller).unwrap();
        fixture.settled().await;
        fixture.assert_completed(&id, 2);
    }
}

async fn updates_keep_history_blocks(mode: StorageMode) {
    for kind in [
        FaultKind::SessionHistoryLoadFailed,
        FaultKind::SessionHistoryWriteFailed,
    ] {
        for candidate in [false, true] {
            let fixture = RecoveryFixture::new(mode, kind);
            let caller = fixture.connection.caller();
            // Isolated existing-hard-block fixture; no save work or damaged
            // bytes are fabricated. Existing storage tests cover reconciliation.
            fixture.session.state.lock().unwrap().block_reason =
                Some(Fault::new(kind, "history remains blocked"));
            let info = fixture.session.info(&caller).unwrap();
            if candidate {
                fixture
                    .session
                    .update_candidate_async(&caller, 0, info.config, None)
                    .await
                    .unwrap();
            } else {
                fixture
                    .session
                    .update_async(&caller, 0, &SessionConfigPatch::default())
                    .await
                    .unwrap();
            }
            assert_eq!(
                fixture
                    .session
                    .state
                    .lock()
                    .unwrap()
                    .block_reason
                    .as_ref()
                    .unwrap()
                    .kind(),
                Some(kind)
            );
            assert_eq!(fixture.resources.attempts.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn resource_resume_memory() {
    resume_reprepares(StorageMode::Memory).await;
}
#[tokio::test]
async fn resource_resume_persistent() {
    resume_reprepares(StorageMode::Persistent).await;
}
#[tokio::test]
async fn resource_patch_memory() {
    update_releases_only_resource_block(StorageMode::Memory, false).await;
}
#[tokio::test]
async fn resource_patch_persistent() {
    update_releases_only_resource_block(StorageMode::Persistent, false).await;
}
#[tokio::test]
async fn resource_candidate_memory() {
    update_releases_only_resource_block(StorageMode::Memory, true).await;
}
#[tokio::test]
async fn resource_candidate_persistent() {
    update_releases_only_resource_block(StorageMode::Persistent, true).await;
}
#[tokio::test]
async fn resource_updates_keep_history_blocks_memory() {
    updates_keep_history_blocks(StorageMode::Memory).await;
}
#[tokio::test]
async fn resource_updates_keep_history_blocks_persistent() {
    updates_keep_history_blocks(StorageMode::Persistent).await;
}

struct GatedPreparation {
    resources: Arc<Resources>,
    entered: Semaphore,
    release: Semaphore,
}
impl ResourceResolver for GatedPreparation {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.resources.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.resources.resolve(config).await
        })
    }
}

#[tokio::test]
async fn real_host_revision_conflict_clears_after_session_update() {
    use crate::{Host, HostConfig, PreparedHostConfiguration};
    let model = model(vec![final_events("new Host revision")]);
    let resources = Arc::new(Resources {
        model: model.clone(),
        tools: ToolSet::default(),
    });
    let gate = Arc::new(GatedPreparation {
        resources: resources.clone(),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let host = Host::memory(gate.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let created = host
        .create_agent_async(
            &caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "configuration race fixture".into(),
                mode: StorageMode::Memory,
                config: AgentConfig::default(),
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    let id = MessageId::new().unwrap();
    host.send(
        &caller,
        SendMessage::new(
            MessageInput {
                message_id: id.clone(),
                mode: MessageMode::FollowUp,
                text: "retain through Host reload".into(),
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
    gate.entered.acquire().await.unwrap().forget();
    host.reload_configuration(
        &caller,
        0,
        PreparedHostConfiguration::new(
            HostConfig {
                name: "new Host revision".into(),
                ..Default::default()
            },
            resources,
            vec![],
            None,
        )
        .unwrap(),
    )
    .await
    .unwrap();
    gate.release.add_permits(1);
    let mut states = host.session_states(&caller, &target).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = states.snapshot().unwrap().data;
            if let Some(block) = state.block_reason {
                assert_eq!(
                    block.fault.unwrap().kind(),
                    Some(FaultKind::SessionConfigConflict)
                );
                assert_eq!(state.next_turn[0].message_id, id);
                assert!(!state.preparing);
                break;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    host.update_session_async(&caller, &target, 0, &SessionConfigPatch::default())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(turn) = host.turns(&caller, &target).unwrap().first() {
                let result = host.wait_turn(&caller, &target, &turn.id).await.unwrap();
                assert_eq!(result.info.outcome, Some(TurnOutcome::Completed));
                assert_eq!(result.info.config_revision, 1);
                break;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        host.message(&caller, &target, &id).unwrap().binding,
        MessageBinding::Bound { .. }
    ));
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
}
