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
    pub(crate) fn load(
        self: &Arc<Self>,
        caller: &CallerContext,
        resume: bool,
    ) -> Result<(), Fault> {
        self.ensure_executor()?;
        let load_id = LoadId::new()
            .map_err(|error| Fault::new(FaultKind::SessionInternalUnexpected, error.to_string()))?;
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            if state.active.is_some() || state.preparing || state.saving {
                return Err(Fault::new(
                    FaultKind::SessionBusy,
                    "Session still owns execution or history work",
                ));
            }
            if let Some(fault) = &state.block_reason
                && fault.kind() == FaultKind::SessionHistoryWriteFailed
            {
                return Err(fault.clone());
            }
            state.preparing = true;
            state.residency = Residency::Loading;
            let pause_generation = state.pause_generation;
            let owner = Arc::clone(self);
            let runtime = state.executor.clone().expect("checked runtime");
            state.load_task = Some(runtime.spawn(async move {
                let result = if owner.backend.mode() == StorageMode::Persistent {
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
                    Ok(owner
                        .state
                        .lock()
                        .expect("Session mutex poisoned")
                        .history
                        .clone())
                };
                {
                    let mut state = owner.state.lock().expect("Session mutex poisoned");
                    state.preparing = false;
                    state.load_task.take();
                    match result {
                        Ok(history) => {
                            state.history = history;
                            state.history_loaded = true;
                            if state.block_reason.as_ref().is_some_and(|fault| {
                                fault.kind() == FaultKind::SessionHistoryLoadFailed
                            }) {
                                state.block_reason = None;
                            }
                            if state.lifecycle == SessionLifecycle::Open {
                                state.load_id.get_or_insert(load_id);
                                state.residency = Residency::Loaded;
                                if resume && state.pause_generation == pause_generation {
                                    state.paused = false;
                                    state.failure_paused = false;
                                }
                            } else {
                                state.residency = Residency::Unloaded;
                                state.load_id = None;
                                if state.info.mode == StorageMode::Memory
                                    && state.lifecycle == SessionLifecycle::Closing
                                {
                                    state.lifecycle = SessionLifecycle::Closed;
                                }
                            }
                        }
                        Err(fault) => {
                            state.residency = Residency::Unloaded;
                            state.load_id = None;
                            state.history_loaded = false;
                            state.block_reason = Some(fault);
                        }
                    }
                    owner.publish(&mut state);
                }
                owner.wake();
            }));
            self.publish(&mut state);
            Ok(())
        })
    }
    fn record(&self, state: &SessionState) -> SessionRecord {
        let mut record = self
            .provenance
            .clone()
            .expect("persistent Session has provenance");
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
        if self.store.is_none() {
            return self.update(caller, expected_revision, patch);
        }
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
        self.resources.validate(&candidate)?;
        self.ensure_executor()?;
        let (sender, receiver) = oneshot::channel();
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            if state.info.config_revision != expected_revision {
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
            let mut record = self.record(&state);
            record.data.config_revision = expected_revision.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "configuration revision exhausted",
                )
            })?;
            record.data.effective_config = candidate;
            self.spawn_record_save(&mut state, record, sender);
            Ok(())
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::SessionInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
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
        state.record_task = Some(state.executor.as_ref().expect("checked runtime").spawn(
            async move {
                let result = owner.save_record(candidate).await;
                let _ = sender.send(result);
            },
        ));
        self.publish(state);
    }
    async fn save_record(self: &Arc<Self>, candidate: SessionRecord) -> Result<SessionInfo, Fault> {
        let store = self.store.clone().expect("persistent Session");
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
            state.record_task.take();
            match result {
                Ok(_) => {
                    state.record_base = Some(candidate.clone());
                    state.info.config = candidate.data.effective_config;
                    state.info.config_revision = candidate.data.config_revision;
                    if candidate.data.closed.is_some() {
                        state.lifecycle = SessionLifecycle::Closed;
                        state.residency = Residency::Unloaded;
                        state.load_id = None;
                    }
                    if state
                        .block_reason
                        .as_ref()
                        .is_some_and(|fault| fault.kind() == FaultKind::SessionResourceMissing)
                    {
                        state.block_reason = None;
                    }
                    state.record_work = None;
                    state.close_fault = None;
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
    pub(crate) fn begin_close_owned(&self) -> Result<(), Fault> {
        let handle = self.close_owned()?;
        if let Some(handle) = handle {
            handle.shutdown();
        }
        Ok(())
    }
    pub(crate) fn set_close_fault(&self, fault: Fault) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.lifecycle == SessionLifecycle::Closed {
            return;
        }
        state.close_fault = Some(fault);
        self.publish(&mut state);
    }
    /// Called only by the direct Agent after its saved default no longer points
    /// here. The Agent owns the wait; this Session owns its record and conclusion.
    pub(crate) async fn finish_close_owned(self: &Arc<Self>) -> Result<(), Fault> {
        self.wait_quiescent_owned().await;
        if self.store.is_none() {
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
        self.save_record(candidate).await.map(|_| ())
    }
}
