use super::*;
use crate::persistence::disk;
use minicore_host_storage::SessionClosure;
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) struct SessionRecordWork {
    candidate: SessionRecord,
    baseline: SessionRecord,
    pub(super) running: bool,
    pub(super) fault: Option<Fault>,
}
impl SessionController {
    pub(crate) fn set_source_revision(&self, revision: ConfigRevision) {
        self.state
            .lock()
            .expect("Session mutex poisoned")
            .info
            .agent_config_revision = revision;
    }
    pub(super) fn start_load_owned(
        self: &Arc<Self>,
        state: &mut SessionState,
        resume: bool,
    ) -> Result<(), Fault> {
        let load_id = LoadId::new()
            .map_err(|error| Fault::new(FaultKind::SessionInternalUnexpected, error.to_string()))?;
        Self::open(state)?;
        if state.active.is_some()
            || state.preparing
            || state.saving
            || state.compaction.is_some()
            || state.history_work.is_some()
        {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "Session still owns execution or history work",
            ));
        }
        if let Some(fault) = &state.block_reason
            && fault.kind() == Some(FaultKind::SessionHistoryWriteFailed)
        {
            return Err(fault.clone());
        }
        state.preparing = true;
        state.residency = Residency::Loading;
        state.resume_after_load = resume.then_some(state.pause_generation);
        let owner = Arc::clone(self);
        let runtime = state.executor.clone().expect("checked runtime");
        let previous = state.load_task.take();
        state.load_task = Some(runtime.spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let persistent = owner.backend.mode() == StorageMode::Persistent;
            let result = if persistent {
                AssertUnwindSafe(owner.backend.load())
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Err(Fault::new(
                            FaultKind::SessionHistoryLoadFailed,
                            "history loader failed unexpectedly",
                        ))
                    })
            } else {
                // Memory load retains the current context and partial content.
                Ok(Vec::new())
            };
            {
                let mut state = owner.state.lock().expect("Session mutex poisoned");
                state.preparing = false;
                let result = result.and_then(|history| {
                    if persistent {
                        state.replace_history(&history)?;
                    }
                    Ok(())
                });
                match result {
                    Ok(()) => {
                        state.history_loaded = true;
                        if state.block_reason.as_ref().is_some_and(|fault| {
                            fault.kind() == Some(FaultKind::SessionHistoryLoadFailed)
                        }) {
                            state.block_reason = None;
                        }
                        if state.lifecycle == SessionLifecycle::Open
                            && !state.host_stopping
                            && owner.access.execution_allowed_owned()
                        {
                            state.load_id.get_or_insert(load_id);
                            state.residency = Residency::Loaded;
                            owner.finish_load_resume(&mut state);
                        } else {
                            state.resume_after_load = None;
                            state.residency = Residency::Unloaded;
                            state.load_id = None;
                            Self::finish_memory_close(&mut state);
                        }
                    }
                    Err(fault) => {
                        state.resume_after_load = None;
                        state.residency = Residency::Unloaded;
                        state.load_id = None;
                        state.history_loaded = false;
                        state.block_reason = Some(fault);
                    }
                }
                owner.publish(&mut state);
            }
            owner.handoff_delegation_owned();
            owner.wake();
        }));
        self.publish(state);
        Ok(())
    }
    fn record(&self, state: &SessionState) -> SessionRecord {
        let mut record = self
            .provenance
            .clone()
            .expect("persistent Session has provenance");
        record.data.title = state.info.title.clone();
        record.data.config_revision = state.info.config_revision;
        record.data.effective_config = state.info.config.clone();
        record
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
        self.spawn_record_save(state, candidate, sender);
    }
    fn spawn_record_save(
        self: &Arc<Self>,
        state: &mut SessionState,
        candidate: SessionRecord,
        sender: oneshot::Sender<Result<SessionInfo, Fault>>,
    ) {
        let baseline = state
            .record_work
            .as_ref()
            .map(|work| work.baseline.clone())
            .or_else(|| state.record_base.clone())
            .expect("persistent metadata baseline");
        state.record_work = Some(SessionRecordWork {
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
        let updated = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.record_work.as_ref().is_none_or(|work| {
                work.candidate != candidate || work.baseline != expected_baseline
            }) {
                return Err(Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "metadata completion does not match its original candidate and baseline",
                ));
            }
            match result {
                Ok(_) => {
                    state.record_base = Some(candidate.clone());
                    state.info.title = candidate.data.title;
                    state.info.config = candidate.data.effective_config;
                    state.info.config_revision = candidate.data.config_revision;
                    if candidate.data.closed.is_some() {
                        state.lifecycle = SessionLifecycle::Closed;
                        state.residency = Residency::Unloaded;
                        state.load_id = None;
                    }
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
                    state.record_work = None;
                    state.close_fault = None;
                    self.finish_save_resume(&mut state);
                    self.publish(&mut state);
                    Ok(state.info.clone())
                }
                Err(fault) => {
                    let work = state.record_work.as_mut().expect("original save retained");
                    work.running = false;
                    work.fault = Some(fault.clone());
                    self.publish(&mut state);
                    Err(fault)
                }
            }
        };
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
            self.spawn_record_save(&mut state, candidate, sender);
            Ok(())
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::SessionInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
    }
    pub(crate) fn begin_close_owned(
        &self,
        operation: OperationId,
    ) -> Result<SessionCloseReceipt, Fault> {
        let (receipt, handle) = self.close_owned(operation)?;
        if let Some(handle) = handle {
            handle.shutdown();
        }
        Ok(receipt)
    }
    pub(crate) fn set_close_fault(&self, fault: Fault) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.close_fault = Some(fault);
        self.publish(&mut state);
    }
    /// Called only by the direct Agent after its saved default no longer points
    /// here. The Agent owns the wait; this Session owns its record and conclusion.
    pub(crate) async fn finish_close_owned(self: &Arc<Self>) -> Result<(), Fault> {
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
                reason: "explicit close".into(),
            });
            state.record_work = Some(SessionRecordWork {
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
    fn accept_update_candidate<T>(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected: u64,
        config: SessionConfig,
        title: Option<String>,
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
        self.resources_owned()?.validate(&config)?;
        self.ensure_executor()?;
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
                self.spawn_record_save(&mut state, record, sender);
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
        let (_, receiver) =
            self.accept_update_candidate(caller, expected, config, title, |_| ())?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::SessionInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
    }
    pub(crate) fn update_control(
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
        let (receipt, _) = self.accept_update_candidate(
            caller,
            request.expected_config_revision,
            config,
            title,
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
