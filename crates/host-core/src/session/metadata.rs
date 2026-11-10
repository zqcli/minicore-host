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
impl SessionController {
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
