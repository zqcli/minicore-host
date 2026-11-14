use super::*;
use crate::persistence::disk;
use minicore_host_storage::SessionClosure;
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) struct SessionRecordWork {
    candidate: SessionRecord,
    baseline: SessionRecord,
    operation: Option<OperationId>,
    pub(super) running: bool,
    pub(super) fault: Option<Fault>,
}
impl SessionRecordWork {
    pub(super) fn activity_only(&self) -> bool {
        let mut baseline = self.baseline.clone();
        baseline.data.last_activity_at = self.candidate.data.last_activity_at;
        baseline == self.candidate
    }
}

impl SessionController {
    /// A metadata checkpoint belongs to the Session owner, never the reader or
    /// caller waiting on it. Reuse the same candidate/baseline/reconciliation path.
    pub(super) fn checkpoint_activity(self: &Arc<Self>) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        self.checkpoint_activity_locked(&mut state);
    }
    pub(super) fn checkpoint_activity_locked(self: &Arc<Self>, state: &mut SessionState) {
        if state.info.mode != StorageMode::Persistent
            || state.record_work.is_some()
            || state.releasing
            || state.released
            || state.reclaimed
            || state.deleting
            || state.lifecycle == SessionLifecycle::Closed
            || state.executor.is_none()
            || state
                .record_base
                .as_ref()
                .is_none_or(|base| base.data.last_activity_at >= state.info.last_activity_at)
        {
            return;
        }
        let candidate = self.record(state);
        let (sender, receiver) = oneshot::channel();
        drop(receiver);
        self.spawn_record_save(state, candidate, sender, None);
    }
    pub(crate) fn set_source_revision(&self, revision: ConfigRevision) {
        self.state
            .lock()
            .expect("Session mutex poisoned")
            .info
            .agent_config_revision = revision;
    }
    fn record(&self, state: &SessionState) -> SessionRecord {
        let mut record = state
            .record_base
            .clone()
            .expect("persistent Session has a confirmed record");
        record.data.created_at = state.info.created_at;
        record.data.last_activity_at = state.info.last_activity_at;
        record.data.title = state.info.title.clone();
        record.data.config_revision = state.info.config_revision;
        record.data.effective_config = state.info.config.clone();
        record
    }
    pub(crate) fn update(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected_revision: u64,
        patch: &SessionConfigPatch,
    ) -> Result<SessionInfo, Fault> {
        let original = self.info(caller)?;
        if original.config_revision != expected_revision {
            return Err(Fault::new(
                FaultKind::SessionConfigConflict,
                "Session configuration revision changed",
            ));
        }
        let candidate = original
            .config
            .apply_patch(patch)
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        self.resources_owned()?.validate(&candidate)?;
        if original.mode == StorageMode::Persistent {
            return self.access.accept_execution(caller, || {
                Self::open(&self.state.lock().expect("Session mutex poisoned"))?;
                Err(Fault::new(
                    FaultKind::HostMethodUnavailable,
                    "persistent configuration updates require the asynchronous owner entry",
                ))
            });
        }
        let (info, _) = self.accept_prepared_update_candidate(
            caller,
            expected_revision,
            candidate,
            original.title,
            None,
            |state| state.info.clone(),
        )?;
        Ok(info)
    }
    pub(crate) async fn update_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected_revision: u64,
        patch: &SessionConfigPatch,
    ) -> Result<SessionInfo, Fault> {
        if self.store_owned().is_none() {
            return self.update(caller, expected_revision, patch);
        }
        let original = self.info(caller)?;
        if original.config_revision != expected_revision {
            return Err(Fault::new(
                FaultKind::SessionConfigConflict,
                "Session configuration revision changed",
            ));
        }
        let config = original
            .config
            .apply_patch(patch)
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        self.update_candidate_async(caller, expected_revision, config, original.title)
            .await
    }
    pub(super) fn record_continuation_candidate(
        state: &SessionState,
    ) -> Result<Option<SessionRecord>, Fault> {
        let Some(work) = &state.record_work else {
            return Ok(None);
        };
        if work.running && work.activity_only() {
            return Ok(None);
        }
        if work.running {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "original metadata save is still running",
            ));
        }
        Ok(Some(work.candidate.clone()))
    }
    pub(super) fn start_record_continuation(
        self: &Arc<Self>,
        state: &mut SessionState,
        candidate: SessionRecord,
    ) {
        let (sender, receiver) = oneshot::channel();
        drop(receiver); // Waiting is not ownership. The accepted owner task persists.
        self.spawn_record_save(state, candidate, sender, None);
    }
    fn spawn_record_save(
        self: &Arc<Self>,
        state: &mut SessionState,
        candidate: SessionRecord,
        sender: oneshot::Sender<Result<SessionInfo, Fault>>,
        operation: Option<OperationId>,
    ) {
        let baseline = state
            .record_work
            .as_ref()
            .map(|work| work.baseline.clone())
            .or_else(|| state.record_base.clone())
            .expect("persistent metadata baseline");
        // Continuation belongs to the original accepted save, not the later
        // caller that explicitly resumes it. Unknown original identity stays unknown.
        let operation = state
            .record_work
            .as_ref()
            .map_or(operation, |work| work.operation.clone());
        state.record_work = Some(SessionRecordWork {
            operation,
            candidate: candidate.clone(),
            baseline,
            running: true,
            fault: None,
        });
        let owner = Arc::clone(self);
        let previous = state.record_task.take();
        state.record_task = Some(state.executor.as_ref().expect("checked runtime").spawn(
            async move {
                if let Some(previous) = previous {
                    let _ = previous.await;
                }
                let result = owner.save_record(candidate).await;
                drop(owner);
                let _ = sender.send(result);
            },
        ));
        self.publish(state);
    }
    async fn save_record(self: &Arc<Self>, candidate: SessionRecord) -> Result<SessionInfo, Fault> {
        let store = self.store_owned().expect("persistent Session");
        let saved = candidate.clone();
        let baseline = self
            .state
            .lock()
            .expect("Session mutex poisoned")
            .record_work
            .as_ref()
            .expect("owned save")
            .baseline
            .clone();
        let expected_baseline = baseline.clone();
        let result = disk(FaultKind::SessionHistoryWriteFailed, move || {
            let current = store.load()?;
            if current != baseline && current != saved {
                return Err(minicore_host_storage::StorageError::UncertainWrite);
            }
            store.save(&saved)
        })
        .await;
        let (updated, record, scheduling) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let previous = state.lifecycle;
            let schedule = diagnostics::Schedule::capture(&state);
            let operation = state
                .record_work
                .as_ref()
                .and_then(|work| work.operation.clone());
            if state.record_work.as_ref().is_none_or(|work| {
                work.candidate != candidate || work.baseline != expected_baseline
            }) {
                return Err(Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "metadata completion does not match its original candidate and baseline",
                ));
            }
            let (updated, record) = match result {
                Ok(_) => {
                    state.record_base = Some(candidate.clone());
                    state.info.last_activity_at = state
                        .info
                        .last_activity_at
                        .max(candidate.data.last_activity_at);
                    state.info.title = candidate.data.title;
                    state.info.config = candidate.data.effective_config;
                    state.info.config_revision = candidate.data.config_revision;
                    if candidate.data.closed.is_some() {
                        state.lifecycle = SessionLifecycle::Closed;
                        state.residency = Residency::Unloaded;
                        state.load_id = None;
                    }
                    if candidate.data.config_revision != expected_baseline.data.config_revision
                        && state.block_reason.as_ref().is_some_and(|fault| {
                            matches!(
                                fault.kind(),
                                Some(
                                    FaultKind::SessionResourceMissing
                                        | FaultKind::SessionConfigInvalid
                                        | FaultKind::SessionConfigConflict
                                )
                            )
                        })
                    {
                        state.block_reason = None;
                    }
                    state.record_work = None;
                    state.close_fault = None;
                    // Register any newer accepted activity before publishing a
                    // quiescent state: release/shutdown cannot slip between saves.
                    self.checkpoint_activity_locked(&mut state);
                    self.finish_save_resume(&mut state);
                    self.publish(&mut state);
                    (Ok(state.info.clone()), self.closed_record(&state, previous))
                }
                Err(fault) => {
                    let work = state.record_work.as_mut().expect("original save retained");
                    work.running = false;
                    work.fault = Some(fault.clone());
                    self.publish(&mut state);
                    let record = diagnostics::metadata_failed(
                        &self.identity(),
                        state
                            .record_work
                            .as_ref()
                            .and_then(|work| work.operation.as_ref()),
                        &fault,
                        state.controller_revision,
                    );
                    (Err(fault), Some(record))
                }
            };
            let scheduling = self.scheduling_record(
                &state,
                schedule,
                "metadata_save_finished",
                operation.as_ref(),
            );
            (updated, record, scheduling)
        };
        diagnostic::emit(record);
        diagnostic::emit(scheduling);
        if updated.is_ok() {
            self.handoff_delegation_owned();
            self.wake();
        }
        updated
    }
    /// Explicit continuation of the same metadata record, never an automatic
    /// retry or a rewrite with a newly observed configuration/version.
    pub(crate) async fn reconcile_record(
        self: &Arc<Self>,
        caller: &CallerContext,
    ) -> Result<SessionInfo, Fault> {
        self.ensure_executor()?;
        let (sender, receiver) = oneshot::channel();
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let work = state.record_work.as_ref().ok_or_else(|| {
                Fault::new(FaultKind::SessionBusy, "there is no pending metadata save")
            })?;
            if work.running {
                return Err(Fault::new(
                    FaultKind::SessionBusy,
                    "metadata save is still running",
                ));
            }
            let candidate = work.candidate.clone();
            self.spawn_record_save(&mut state, candidate, sender, None);
            Ok(())
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::SessionInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
    }
    /// Called only by the direct Agent after its saved default no longer points
    /// here. The Agent owns the wait; this Session owns its record and conclusion.
    pub(crate) async fn finish_close_owned(
        self: &Arc<Self>,
        operation: Option<&OperationId>,
    ) -> Result<(), Fault> {
        self.wait_quiescent_owned().await;
        if self.store_owned().is_none() {
            return Ok(());
        }
        let candidate = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.lifecycle == SessionLifecycle::Closed {
                return Ok(());
            }
            if let Some(work) = &state.record_work {
                return Err(work.fault.clone().unwrap_or_else(|| {
                    Fault::new(FaultKind::SessionBusy, "metadata save is pending")
                }));
            }
            let mut candidate = self.record(&state);
            candidate.data.closed = Some(SessionClosure {
                reason: state.end_reason.clone().expect("accepted close reason"),
            });
            state.record_work = Some(SessionRecordWork {
                operation: operation.cloned(),
                baseline: state.record_base.clone().expect("metadata baseline"),
                candidate: candidate.clone(),
                running: true,
                fault: None,
            });
            self.publish(&mut state);
            candidate
        };
        self.save_record(candidate).await?;
        self.scope_root_closed_owned();
        Ok(())
    }
}

impl SessionController {
    fn accept_prepared_update_candidate<T>(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected: u64,
        config: SessionConfig,
        title: Option<String>,
        operation: Option<OperationId>,
        accepted: impl FnOnce(&mut SessionState) -> T,
    ) -> Result<(T, oneshot::Receiver<Result<SessionInfo, Fault>>), Fault> {
        if title
            .as_ref()
            .is_some_and(|title| title.trim().is_empty() || title.contains('\0'))
        {
            return Err(Fault::new(
                FaultKind::SessionConfigInvalid,
                "Session title must not be blank",
            ));
        }
        config
            .validate()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        if self.storage_mode() == StorageMode::Persistent {
            self.ensure_executor()?;
        }
        let (sender, receiver) = oneshot::channel();
        let result = self.access.accept_execution(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            if state.info.config_revision != expected {
                return Err(Fault::new(
                    FaultKind::SessionConfigConflict,
                    "Session configuration revision changed",
                ));
            }
            if state.record_work.is_some() {
                return Err(Fault::new(
                    FaultKind::SessionBusy,
                    "Session has a pending metadata save",
                ));
            }
            let revision = expected.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "configuration revision exhausted",
                )
            })?;
            if state.info.mode == StorageMode::Memory {
                state.info.config = config;
                state.info.title = title;
                state.info.config_revision = revision;
                if state.block_reason.as_ref().is_some_and(|fault| {
                    matches!(
                        fault.kind(),
                        Some(
                            FaultKind::SessionResourceMissing
                                | FaultKind::SessionConfigInvalid
                                | FaultKind::SessionConfigConflict
                        )
                    )
                }) {
                    state.block_reason = None;
                }
                self.publish(&mut state);
                let _ = sender.send(Ok(state.info.clone()));
            } else {
                let mut record = self.record(&state);
                record.data.effective_config = config;
                record.data.title = title;
                record.data.config_revision = revision;
                self.spawn_record_save(&mut state, record, sender, operation);
            }
            Ok(accepted(&mut state))
        })?;
        self.wake();
        Ok((result, receiver))
    }
    pub(crate) async fn update_candidate_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected: u64,
        config: SessionConfig,
        title: Option<String>,
    ) -> Result<SessionInfo, Fault> {
        self.resources_owned()?.validate(&config)?;
        let (_, receiver) =
            self.accept_prepared_update_candidate(caller, expected, config, title, None, |_| ())?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::SessionInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
    }
    pub(crate) async fn update_control(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: SessionUpdateRequest,
    ) -> Result<SessionUpdateReceipt, Fault> {
        request
            .changes
            .validate()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        let original = self.info(caller)?;
        if original.config_revision != request.expected_config_revision {
            return Err(Fault::new(
                FaultKind::SessionConfigConflict,
                "Session configuration revision changed",
            ));
        }
        let config = original
            .config
            .apply_patch(&request.changes.config)
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        let title = match request.changes.title {
            NullablePatch::Unchanged => original.title,
            NullablePatch::Set(title) => Some(title),
            NullablePatch::Clear => None,
        };
        self.ensure_executor()?;
        let resources = self.resources_owned()?;
        let candidate = config.clone();
        self.executor()
            .expect("checked runtime")
            .spawn_blocking(move || resources.validate(&candidate))
            .await
            .map_err(|_| {
                Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "configuration validation task ended unexpectedly",
                )
            })??;
        let (receipt, _) = self.accept_prepared_update_candidate(
            caller,
            request.expected_config_revision,
            config,
            title,
            Some(request.operation_id.clone()),
            |state| {
                let result = if state.record_work.is_some() {
                    MetadataUpdateResult::Saving {
                        config_revision: state.info.config_revision,
                    }
                } else {
                    MetadataUpdateResult::Committed {
                        config_revision: state.info.config_revision,
                    }
                };
                let receipt = SessionUpdateReceipt {
                    kind: SessionUpdateKind::SessionUpdate,
                    host_id: self.host_id.clone(),
                    host_incarnation: self.incarnation.clone(),
                    agent_id: self.agent_id.clone(),
                    session_id: self.id.clone(),
                    operation_id: request.operation_id,
                    result,
                };
                Self::retain_control(state, Receipt::SessionUpdate(receipt.clone()));
                receipt
            },
        )?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use crate::{BoundResources, ResourceFuture};
    use minicore_host_storage::{AgentData, AgentRecord, DataRoot, HostRecord, SessionData};
    use std::time::Duration;

    /// This fixture deliberately never executes a model; it exercises paused
    /// input and the production Session metadata writer only.
    struct IdleResources;
    impl ResourceResolver for IdleResources {
        fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
            Ok(())
        }
        fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
            Box::pin(async {
                Err::<BoundResources, _>(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "idle metadata fixture has no model",
                ))
            })
        }
    }
    fn fixture() -> (
        tempfile::TempDir,
        Arc<SessionController>,
        SessionStore,
        crate::ConnectionHandle,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = DataRoot::acquire(dir.path()).unwrap();
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
                    configuration_source: None,
                    name: "activity metadata fixture".into(),
                    mode: StorageMode::Persistent,
                    config: AgentConfig::default(),
                    config_revision: 0,
                    default_session_id: None,
                },
            ))
            .unwrap();
        let id = SessionId::new().unwrap();
        let store = agent.session(id.clone(), StorageMode::Persistent).unwrap();
        let created_at = Timestamp::from_unix_milliseconds(0).unwrap();
        let record = SessionRecord::new(
            id,
            agent_id,
            SessionData {
                created_at,
                last_activity_at: created_at,
                title: None,
                mode: StorageMode::Persistent,
                agent_config_revision: 0,
                config_revision: 0,
                effective_config: SessionConfig::default(),
                parent: None,
                closed: None,
            },
        );
        store.save(&record).unwrap();
        let incarnation = HostIncarnation::new().unwrap();
        let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
        let connection = access.connect().unwrap();
        let owner = SessionController::from_record(
            record,
            store.clone(),
            host_id,
            incarnation,
            access,
            Arc::new(IdleResources),
            Arc::new(AtomicBool::new(true)),
            true,
        );
        owner.state.lock().unwrap().paused = true;
        (dir, owner, store, connection)
    }
    async fn settled(owner: &SessionController) {
        let mut changes = owner.snapshots.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if owner.state.lock().unwrap().record_work.is_none() {
                    return;
                }
                changes.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    fn message() -> MessageInput {
        MessageInput {
            message_id: MessageId::new().unwrap(),
            mode: MessageMode::FollowUp,
            text: "accepted while paused".into(),
            in_reply_to: None,
        }
    }

    #[tokio::test]
    async fn newer_activity_coalesces_before_quiescence_and_survives_shutdown() {
        let (_dir, owner, store, connection) = fixture();
        // Deterministic gate between original metadata acceptance and worker I/O.
        // This is a named scheduling fixture, not a simulated disk confirmation.
        let candidate = {
            let mut state = owner.state.lock().unwrap();
            let mut candidate = owner.record(&state);
            candidate.data.title = Some("original accepted title".into());
            candidate.data.config_revision = 1;
            state.record_work = Some(SessionRecordWork {
                candidate: candidate.clone(),
                baseline: state.record_base.clone().unwrap(),
                operation: None,
                running: true,
                fault: None,
            });
            candidate
        };
        owner.send(&connection.caller(), message()).unwrap();
        let received_at = owner.state.lock().unwrap().messages[0].received_at;
        assert!(received_at > candidate.data.last_activity_at);
        assert_eq!(
            owner
                .state
                .lock()
                .unwrap()
                .record_work
                .as_ref()
                .unwrap()
                .candidate,
            candidate
        );
        owner.begin_host_shutdown_owned(true);
        assert!(!owner.shutdown_status_owned().quiescent);
        owner.save_record(candidate).await.unwrap();
        settled(&owner).await;
        let saved = store.load().unwrap();
        assert_eq!(saved.data.last_activity_at, received_at);
        assert_eq!(saved.data.title.as_deref(), Some("original accepted title"));
        assert_eq!(saved.data.config_revision, 1);
        assert_eq!(
            saved.data.created_at,
            Timestamp::from_unix_milliseconds(0).unwrap()
        );
        assert!(owner.state.lock().unwrap().next_turn.is_empty());
        assert!(!owner.state.lock().unwrap().history_loaded);
        assert!(store.read_history().unwrap().records.is_empty());
        let before = store.load().unwrap();
        owner.info(&connection.caller()).unwrap();
        owner.summary(&connection.caller()).unwrap();
        owner
            .history_read(
                &connection.caller(),
                &HistoryReadRequest {
                    cursor: None,
                    limit: 3,
                    direction: HistoryDirection::Backward,
                },
            )
            .await
            .unwrap();
        assert_eq!(store.load().unwrap(), before);
        assert_eq!(
            owner.info(&connection.caller()).unwrap().last_activity_at,
            received_at
        );
        owner.release_host_resources_owned().await.unwrap();
        owner.checkpoint_activity();
        assert!(owner.state.lock().unwrap().record_work.is_none());
        assert!(owner.state.lock().unwrap().record_task.is_none());

        let incarnation = HostIncarnation::new().unwrap();
        let access = ConnectionRegistry::new(owner.host_id.clone(), incarnation.clone());
        let reopened_connection = access.connect().unwrap();
        let reopened = SessionController::from_record(
            saved,
            store,
            owner.host_id.clone(),
            incarnation,
            access,
            Arc::new(IdleResources),
            Arc::new(AtomicBool::new(true)),
            true,
        );
        let summary = reopened
            .summary(&reopened_connection.caller())
            .unwrap()
            .unwrap();
        assert_eq!(summary.last_activity_at, received_at);
        assert_eq!(summary.title.as_deref(), Some("original accepted title"));
        assert!(!reopened.state.lock().unwrap().history_loaded);
    }

    #[tokio::test]
    async fn paused_input_registers_activity_save_before_force_clear() {
        let (_dir, owner, store, connection) = fixture();
        owner.send(&connection.caller(), message()).unwrap();
        let received_at = owner.state.lock().unwrap().messages[0].received_at;
        owner.begin_host_shutdown_owned(true);
        settled(&owner).await;
        assert_eq!(store.load().unwrap().data.last_activity_at, received_at);
        assert!(store.read_history().unwrap().records.is_empty());
        owner.release_host_resources_owned().await.unwrap();
    }

    #[tokio::test]
    async fn recovered_activity_is_checkpointed_before_load_or_preparation_quiesces() {
        for explicit_load in [true, false] {
            let (_dir, owner, store, connection) = fixture();
            // Durable history can be newer than session.json after a previous
            // process ended between the original history and metadata saves.
            // The future value also proves we compare actual timestamps rather
            // than overwriting recovered activity with the current clock.
            let recorded_at = Timestamp::from_unix_milliseconds(4_102_444_800_000).unwrap();
            let mut journal = store.open_journal().unwrap();
            journal
                .accept_batch(vec![minicore_host_storage::HistoryRecord::at(
                    owner.id.clone(),
                    0,
                    recorded_at,
                    HistoryEntry::Summary {
                        summary: minicore_host_runtime::SummaryHistory {
                            content: minicore_host_runtime::Text::new("durable original content"),
                        },
                    },
                )])
                .unwrap();
            journal.persist_pending().unwrap();
            drop(journal);
            if explicit_load {
                owner
                    .control(
                        &connection.caller(),
                        SessionControlRequest {
                            operation_id: OperationId::new().unwrap(),
                        },
                        SessionControlKind::Load,
                    )
                    .unwrap();
                owner.begin_host_shutdown_owned(true);
            } else {
                owner.send(&connection.caller(), message()).unwrap();
                owner.resume(&connection.caller()).unwrap();
                let mut changes = owner.snapshots.subscribe();
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let finished = {
                            let state = owner.state.lock().unwrap();
                            state.history_loaded
                                && !state.driver_running
                                && state.block_reason.is_some()
                        };
                        if finished {
                            break;
                        }
                        changes.changed().await.unwrap();
                    }
                })
                .await
                .unwrap();
                owner.begin_host_shutdown_owned(true);
            }
            owner.wait_quiescent_owned().await;
            owner.release_host_resources_owned().await.unwrap();
            assert_eq!(store.load().unwrap().data.last_activity_at, recorded_at);
            assert_eq!(
                store.read_history().unwrap().records[0].recorded_at,
                recorded_at
            );
            assert!(owner.state.lock().unwrap().record_work.is_none());
            owner.checkpoint_activity();
            assert!(owner.state.lock().unwrap().record_task.is_none());
        }
    }
}
