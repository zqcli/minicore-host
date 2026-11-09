use super::*;
use crate::persistence::disk;
use minicore_host_storage::AgentData;
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) struct AgentRecordWork {
    candidate: AgentRecord,
    baseline: AgentRecord,
    runtime_default: Option<SessionId>,
    running: bool,
    fault: Option<Fault>,
}
impl AgentOwner {
    fn record(&self, info: &AgentInfo, persistent_default: Option<SessionId>) -> AgentRecord {
        AgentRecord::new(
            self.id.clone(),
            self.host_id.clone(),
            AgentData {
                name: info.name.clone(),
                mode: info.mode,
                config: info.config.clone(),
                config_revision: info.config_revision,
                default_session_id: persistent_default,
            },
        )
    }
    fn persisted_default(&self, id: &Option<SessionId>) -> Result<Option<SessionId>, Fault> {
        let Some(id) = id.as_ref() else {
            return Ok(None);
        };
        // Discovery can retain an unavailable persistent default. An unrelated
        // config update must not silently clear that identity because loading
        // its child failed; only an explicit default change may replace it.
        match self.session(id) {
            Ok(child) if child.storage_mode() == StorageMode::Memory => Ok(None),
            Ok(_) => Ok(Some(id.clone())),
            Err(_) => Err(Fault::new(
                FaultKind::AgentDefaultSessionUnavailable,
                "the unavailable default must be explicitly changed before metadata can save",
            )),
        }
    }
    pub(super) fn writable(state: &AgentState) -> Result<(), Fault> {
        if !matches!(
            state.info.lifecycle,
            AgentLifecycle::Active | AgentLifecycle::Stopped
        ) {
            return Err(Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "Agent metadata cannot change while creation or stop is still running",
            ));
        }
        if state.record_work.is_some() {
            return Err(Fault::new(
                FaultKind::AgentConfigConflict,
                "Agent has a pending metadata save",
            ));
        }
        Ok(())
    }
    pub(crate) async fn update_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected: u64,
        patch: &AgentConfigPatch,
    ) -> Result<AgentInfo, Fault> {
        if self.store().is_none() {
            return self.update(caller, expected, patch);
        }
        let original = self.info(caller)?;
        if original.config_revision != expected {
            return Err(Fault::new(
                FaultKind::AgentConfigConflict,
                "Agent configuration revision changed",
            ));
        }
        let candidate = original
            .config
            .apply_patch(patch)
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        self.resources_owned()?.validate(
            &SessionConfig::from_agent(&candidate, &SessionOverrides::default())
                .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?,
        )?;
        let persistent_default = self.persisted_default(&original.default_session_id)?;
        let (sender, receiver) = oneshot::channel();
        self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            Self::writable(&state)?;
            if state.info.config_revision != expected
                || state.info.default_session_id != original.default_session_id
            {
                return Err(Fault::new(
                    FaultKind::AgentConfigConflict,
                    "Agent configuration or default changed during preparation",
                ));
            }
            let mut info = state.info.clone();
            info.config = candidate;
            info.config_revision = expected.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "configuration revision exhausted",
                )
            })?;
            let record = self.record(&info, persistent_default);
            self.spawn_record_save(&mut state, record, info.default_session_id, sender)?;
            Ok(())
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::AgentInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
    }
    pub(crate) async fn set_default_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        target: Option<SessionId>,
    ) -> Result<(), Fault> {
        if self.store().is_none() {
            return self.set_default(caller, target);
        }
        let child = target.as_ref().map(|id| self.session(id)).transpose()?;
        if child.as_ref().is_some_and(|child| !child.is_open_owned()) {
            return Err(Fault::new(
                FaultKind::AgentDefaultSessionUnavailable,
                "default Session must be open",
            ));
        }
        let persistent_default = child
            .filter(|child| child.storage_mode() == StorageMode::Persistent)
            .map(|child| child.id.clone());
        let (sender, receiver) = oneshot::channel();
        self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            Self::writable(&state)?;
            if target.as_ref().is_some_and(|id| state.closing.contains(id)) {
                return Err(Fault::new(
                    FaultKind::AgentDefaultSessionUnavailable,
                    "default Session is closing",
                ));
            }
            let record = self.record(&state.info, persistent_default);
            self.spawn_record_save(&mut state, record, target, sender)?;
            Ok(())
        })?;
        receiver
            .await
            .map_err(|_| {
                Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "metadata owner ended without a result",
                )
            })?
            .map(|_| ())
    }
    fn spawn_record_save(
        self: &Arc<Self>,
        state: &mut AgentState,
        candidate: AgentRecord,
        runtime_default: Option<SessionId>,
        sender: oneshot::Sender<Result<AgentInfo, Fault>>,
    ) -> Result<(), Fault> {
        let runtime = state
            .executor
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::AgentLifecycleUnavailable,
                    "metadata save requires a live Tokio runtime",
                )
            })?;
        state.info.metadata_saving = true;
        state.info.metadata_fault = None;
        let baseline = state
            .record_work
            .as_ref()
            .map(|work| work.baseline.clone())
            .or_else(|| state.record_base.clone())
            .expect("persistent metadata baseline");
        state.record_work = Some(AgentRecordWork {
            candidate: candidate.clone(),
            baseline,
            runtime_default: runtime_default.clone(),
            running: true,
            fault: None,
        });
        let owner = Arc::clone(self);
        state.record_task = Some(runtime.spawn(async move {
            let result = owner.save_record(candidate, runtime_default).await;
            drop(owner);
            let _ = sender.send(result);
        }));
        Ok(())
    }
    async fn save_record(
        self: &Arc<Self>,
        candidate: AgentRecord,
        runtime_default: Option<SessionId>,
    ) -> Result<AgentInfo, Fault> {
        let store = self.store().expect("persistent Agent");
        let saved = candidate.clone();
        let baseline = self
            .state
            .lock()
            .expect("Agent mutex poisoned")
            .record_work
            .as_ref()
            .expect("owned save")
            .baseline
            .clone();
        let result = disk(FaultKind::HostStorageUnavailable, move || {
            let current = store.load_record()?;
            if current != baseline && current != saved {
                return Err(minicore_host_storage::StorageError::UncertainWrite);
            }
            store.save(&saved)
        })
        .await;
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        state.record_task.take();
        state.info.metadata_saving = false;
        let result = match result {
            Ok(_) => {
                state.record_base = Some(candidate.clone());
                state.info.name = candidate.data.name;
                state.info.config = candidate.data.config;
                state.info.config_revision = candidate.data.config_revision;
                state.info.default_session_id = runtime_default;
                state.info.metadata_fault = None;
                state.record_work = None;
                Ok(state.info.clone())
            }
            Err(fault) => {
                let work = state.record_work.as_mut().expect("original save retained");
                work.running = false;
                work.fault = Some(fault.clone());
                state.info.metadata_fault = Some(fault.clone());
                Err(fault)
            }
        };
        self.changed.notify_waiters();
        result
    }
    pub(crate) async fn reconcile_record(
        self: &Arc<Self>,
        caller: &CallerContext,
    ) -> Result<AgentInfo, Fault> {
        let (sender, receiver) = oneshot::channel();
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            let work = state.record_work.as_ref().ok_or_else(|| {
                Fault::new(
                    FaultKind::AgentConfigConflict,
                    "there is no pending metadata save",
                )
            })?;
            if work.running {
                return Err(Fault::new(
                    FaultKind::AgentConfigConflict,
                    "metadata save is still running",
                ));
            }
            let candidate = work.candidate.clone();
            let target = work.runtime_default.clone();
            self.spawn_record_save(&mut state, candidate, target, sender)
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::AgentInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
    }
    pub(crate) fn close_session(
        self: &Arc<Self>,
        caller: &CallerContext,
        id: &SessionId,
    ) -> Result<(), Fault> {
        let child = self.session(id)?;
        if self.store().is_none() {
            self.access.accept_execution(caller, || {
                self.state
                    .lock()
                    .expect("Agent mutex poisoned")
                    .closing
                    .insert(id.clone());
                Ok(())
            })?;
            child.begin_close_owned()?;
            self.clear_default_if(id);
            return Ok(());
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "close requires a live Tokio runtime",
            )
        })?;
        let first = self.access.accept_execution(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            state.closing.insert(id.clone());
            Ok(state.close_running.insert(id.clone()))
        })?;
        child.begin_close_owned()?;
        if !first {
            return Ok(());
        }
        // No owner/access lock is held while asking the child to close or doing I/O.
        let owner = Arc::clone(self);
        let id = id.clone();
        let task_id = id.clone();
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        let task = runtime.spawn(async move {
            let result = async {
                owner.clear_default_before_close(&id).await?;
                child.finish_close_owned().await
            }
            .await;
            {
                let mut state = owner.state.lock().expect("Agent mutex poisoned");
                state.close_tasks.remove(&id);
                state.close_running.remove(&id);
            }
            // Publish a retryable close failure only after this attempt has
            // released its running registration. An observer can then continue
            // the original record and request close again without losing it.
            if let Err(fault) = result {
                child.set_close_fault(fault);
            }
            owner.changed.notify_waiters();
        });
        state.close_tasks.insert(task_id, task);
        Ok(())
    }
    async fn clear_default_before_close(self: &Arc<Self>, id: &SessionId) -> Result<(), Fault> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let candidate = {
                let mut state = self.state.lock().expect("Agent mutex poisoned");
                let references_child = state.info.default_session_id.as_ref() == Some(id)
                    || state
                        .record_work
                        .as_ref()
                        .is_some_and(|work| work.runtime_default.as_ref() == Some(id));
                if !references_child {
                    // An unrelated record candidate does not own this child's
                    // close. In particular, a prior failed Agent config save
                    // must not block healthy nondefault Session closure.
                    return Ok(());
                }
                if let Some(work) = &state.record_work {
                    if let Some(fault) = &work.fault {
                        return Err(fault.clone());
                    }
                    None
                } else if state.info.default_session_id.as_ref() != Some(id) {
                    return Ok(());
                } else {
                    let record = self.record(&state.info, None);
                    state.info.metadata_saving = true;
                    state.record_work = Some(AgentRecordWork {
                        baseline: state.record_base.clone().expect("metadata baseline"),
                        candidate: record.clone(),
                        runtime_default: None,
                        running: true,
                        fault: None,
                    });
                    Some(record)
                }
            };
            if let Some(candidate) = candidate {
                self.save_record(candidate, None).await?;
                return Ok(());
            }
            notified.await;
        }
    }
}
