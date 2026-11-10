use super::*;
use crate::persistence::disk;
use minicore_host_diagnostics::Context;
use minicore_host_storage::AgentData;
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) struct AgentRecordWork {
    candidate: AgentRecord,
    baseline: AgentRecord,
    runtime_default: Option<SessionId>,
    running: bool,
    fault: Option<Fault>,
    context: Context,
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
        let config = original
            .config
            .apply_patch(patch)
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        self.update_candidate_async(caller, expected, config, original.name)
            .await
    }
    pub(crate) async fn set_default_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        target: Option<SessionId>,
    ) -> Result<(), Fault> {
        let (_, receiver) = self.accept_default(caller, target, None, |_| ())?;
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
    fn accept_default<T>(
        self: &Arc<Self>,
        caller: &CallerContext,
        target: Option<SessionId>,
        operation_id: Option<OperationId>,
        accepted: impl FnOnce(&mut AgentState) -> T,
    ) -> Result<(T, oneshot::Receiver<Result<AgentInfo, Fault>>), Fault> {
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
        let context = Context {
            operation_id,
            ..self.diagnostic_context()
        };
        let (result, diagnostic) = self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            Self::writable(&state)?;
            if target
                .as_ref()
                .is_some_and(|id| !state.sessions.contains_key(id))
            {
                return Err(Fault::new(
                    FaultKind::AgentSessionNotFound,
                    "Session does not belong to this Agent",
                ));
            }
            if target.as_ref().is_some_and(|id| state.closing.contains(id)) {
                return Err(Fault::new(
                    FaultKind::AgentDefaultSessionUnavailable,
                    "default Session is closing",
                ));
            }
            let diagnostic = if state.info.mode == StorageMode::Memory {
                state.info.default_session_id = target;
                let _ = sender.send(Ok(state.info.clone()));
                self.changed.notify_waiters();
                self.metadata_result_record(context, &state.info, None)
            } else {
                let record = self.record(&state.info, persistent_default);
                self.spawn_record_save(&mut state, record, target, sender, context.clone())?;
                self.metadata_accepted_record(context, &state.info)
            };
            Ok((accepted(&mut state), diagnostic))
        })?;
        diagnostic.emit();
        Ok((result, receiver))
    }
    pub(crate) fn set_default_control(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: AgentSetDefaultSessionRequest,
    ) -> Result<AgentSetDefaultSessionReceipt, Fault> {
        let requested = request.session_id.clone();
        let (receipt, _) = self.accept_default(
            caller,
            request.session_id,
            Some(request.operation_id.clone()),
            |state| {
                let result = if state.info.metadata_saving {
                    AgentSetDefaultSessionResult::Saving {
                        default_session_id: state.info.default_session_id.clone(),
                        requested_session_id: requested,
                    }
                } else {
                    AgentSetDefaultSessionResult::Committed {
                        default_session_id: state.info.default_session_id.clone(),
                    }
                };
                let receipt = AgentSetDefaultSessionReceipt {
                    kind: AgentSetDefaultSessionKind::AgentSetDefaultSession,
                    host_id: self.host_id.clone(),
                    host_incarnation: self.incarnation.clone(),
                    agent_id: self.id.clone(),
                    operation_id: request.operation_id,
                    result,
                };
                Self::retain_control(state, Receipt::AgentSetDefaultSession(receipt.clone()));
                receipt
            },
        )?;
        Ok(receipt)
    }
    fn spawn_record_save(
        self: &Arc<Self>,
        state: &mut AgentState,
        candidate: AgentRecord,
        runtime_default: Option<SessionId>,
        sender: oneshot::Sender<Result<AgentInfo, Fault>>,
        context: Context,
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
            context,
        });
        let owner = Arc::clone(self);
        state.record_task = Some(
            runtime.spawn(
                async move {
                    let result = owner.save_record(candidate, runtime_default).await;
                    drop(owner);
                    let _ = sender.send(result);
                }
                .with_current_subscriber(),
            ),
        );
        Ok(())
    }
    async fn save_record(
        self: &Arc<Self>,
        candidate: AgentRecord,
        runtime_default: Option<SessionId>,
    ) -> Result<AgentInfo, Fault> {
        let store = self.store().expect("persistent Agent");
        let saved = candidate.clone();
        let (baseline, context) = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            let work = state.record_work.as_ref().expect("owned save");
            (work.baseline.clone(), work.context.clone())
        };
        // This owned worker creates and destroys its short span outside every
        // admission and owner guard, including a failed caller or reconciliation.
        let span = context.span();
        async move {
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
            let diagnostic =
                self.metadata_result_record(context, &state.info, result.as_ref().err());
            self.changed.notify_waiters();
            drop(state);
            diagnostic.emit();
            result
        }
        .instrument(span)
        .await
    }
    pub(crate) async fn reconcile_record(
        self: &Arc<Self>,
        caller: &CallerContext,
    ) -> Result<AgentInfo, Fault> {
        let (sender, receiver) = oneshot::channel();
        let diagnostic = self.access.accept(caller, || {
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
            let context = work.context.clone();
            self.spawn_record_save(&mut state, candidate, target, sender, context.clone())?;
            Ok(self.metadata_accepted_record(context, &state.info))
        })?;
        diagnostic.emit();
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
        request: SessionCloseRequest,
    ) -> Result<SessionCloseReceipt, Fault> {
        let operation = request.operation_id.clone();
        let persistent = self.store().is_some();
        let runtime = if persistent {
            Some(tokio::runtime::Handle::try_current().map_err(|_| {
                Fault::new(
                    FaultKind::AgentLifecycleUnavailable,
                    "close requires a live Tokio runtime",
                )
            })?)
        } else {
            None
        };
        let mut diagnostics = Vec::new();
        let accepted = self.access.accept_execution(caller, || {
            let (child, first) = {
                let mut state = self.state.lock().expect("Agent mutex poisoned");
                if state.deleting {
                    return Err(Fault::new(
                        FaultKind::AgentLifecycleUnavailable,
                        "Agent deletion has fenced new external close requests",
                    ));
                }
                if state.deleting_sessions.contains(id) {
                    return Err(Fault::new(
                        FaultKind::SessionBusy,
                        "Session deletion already owns its cleanup",
                    ));
                }
                if let Some(fault) = state.load_failures.get(id) {
                    return Err(fault.clone());
                }
                let child = state.sessions.get(id).cloned().ok_or_else(|| {
                    Fault::new(
                        FaultKind::AgentSessionNotFound,
                        "Session does not belong to this Agent",
                    )
                })?;
                let first = state.close_running.insert(id.clone());
                (child, first)
            };
            // Registration precedes the child call; no Agent state lock spans it.
            let receipt = match child.begin_close_owned(request.operation_id) {
                Ok((receipt, records)) => {
                    diagnostics.extend(records);
                    receipt
                }
                Err(error) => {
                    if first {
                        self.state
                            .lock()
                            .expect("Agent mutex poisoned")
                            .close_running
                            .remove(id);
                    }
                    return Err(error);
                }
            };
            self.state
                .lock()
                .expect("Agent mutex poisoned")
                .closing
                .insert(id.clone());
            Ok((child, first, receipt))
        });
        crate::diagnostics::emit(diagnostics);
        let (child, first, receipt) = accepted?;
        child.handoff_delegation_owned();
        if !first {
            return Ok(receipt);
        }
        if !persistent {
            self.clear_default_if(id);
            if child.retire_closed_immediately_owned() == Ok(true) {
                self.remove_retired_session_owned(&child);
            }
            self.state
                .lock()
                .expect("Agent mutex poisoned")
                .close_running
                .remove(id);
            self.changed.notify_waiters();
            self.try_retire_empty_owned();
            return Ok(receipt);
        }
        let owner = Arc::clone(self);
        let id = id.clone();
        let task_id = id.clone();
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        let task = runtime.expect("persistent close runtime").spawn(
            async move {
                let result = async {
                    owner.clear_default_before_close(&id, &operation).await?;
                    child.finish_close_owned().await?;
                    child.join_close_owned().await?;
                    if child.storage_mode() == StorageMode::Memory {
                        child.retire_closed_owned().await?;
                        owner.remove_retired_session_owned(&child);
                    }
                    Ok::<_, Fault>(())
                }
                .await;
                {
                    let mut state = owner.state.lock().expect("Agent mutex poisoned");
                    state.close_tasks.remove(&id);
                    state.close_running.remove(&id);
                }
                if let Err(fault) = result {
                    child.set_close_fault(fault);
                }
                owner.changed.notify_waiters();
            }
            .with_current_subscriber(),
        );
        state.close_tasks.insert(task_id, task);
        Ok(receipt)
    }
    pub(super) async fn clear_default_before_close(
        self: &Arc<Self>,
        id: &SessionId,
        operation: &OperationId,
    ) -> Result<(), Fault> {
        let context = Context {
            operation_id: Some(operation.clone()),
            session_id: Some(id.clone()),
            ..self.diagnostic_context()
        };
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
                        context: context.clone(),
                    });
                    Some((
                        record,
                        self.metadata_accepted_record(context.clone(), &state.info),
                    ))
                }
            };
            if let Some((candidate, diagnostic)) = candidate {
                diagnostic.emit();
                self.save_record(candidate, None).await?;
                return Ok(());
            }
            notified.await;
        }
    }
}

impl AgentOwner {
    fn accept_prepared_update_candidate<T>(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected: u64,
        candidate: (AgentConfig, String),
        operation_id: Option<OperationId>,
        accepted: impl FnOnce(&mut AgentState) -> T,
    ) -> Result<(T, oneshot::Receiver<Result<AgentInfo, Fault>>), Fault> {
        let (config, name) = candidate;
        if name.trim().is_empty() || name.contains('\0') {
            return Err(Fault::new(
                FaultKind::AgentConfigInvalid,
                "Agent name must not be blank",
            ));
        }
        let original = self.info(caller)?;
        if original.config_revision != expected {
            return Err(Fault::new(
                FaultKind::AgentConfigConflict,
                "Agent configuration revision changed",
            ));
        }
        let persistent_default = if self.store().is_some() {
            self.persisted_default(&original.default_session_id)?
        } else {
            None
        };
        let (sender, receiver) = oneshot::channel();
        let context = Context {
            operation_id,
            ..self.diagnostic_context()
        };
        let (result, diagnostic) = self.access.accept_business(caller, || {
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
            let mut candidate = state.info.clone();
            candidate.name = name;
            candidate.config = config;
            candidate.config_revision = expected.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "configuration revision exhausted",
                )
            })?;
            let diagnostic = if state.info.mode == StorageMode::Memory {
                state.info = candidate;
                self.changed.notify_waiters();
                let _ = sender.send(Ok(state.info.clone()));
                self.metadata_result_record(context, &state.info, None)
            } else {
                let record = self.record(&candidate, persistent_default);
                self.spawn_record_save(
                    &mut state,
                    record,
                    candidate.default_session_id,
                    sender,
                    context.clone(),
                )?;
                self.metadata_accepted_record(context, &state.info)
            };
            Ok((accepted(&mut state), diagnostic))
        })?;
        diagnostic.emit();
        Ok((result, receiver))
    }
    pub(crate) async fn update_candidate_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected: u64,
        config: AgentConfig,
        name: String,
    ) -> Result<AgentInfo, Fault> {
        self.resources_owned()?
            .validate(
                &SessionConfig::from_agent(&config, &SessionOverrides::default()).map_err(
                    |error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()),
                )?,
            )
            .inspect_err(|fault| self.metadata_refusal_record(None, fault).emit())?;
        let (_, receiver) =
            self.accept_prepared_update_candidate(caller, expected, (config, name), None, |_| ())?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::AgentInternalUnexpected,
                "metadata owner ended without a result",
            )
        })?
    }
    pub(crate) async fn update_control(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: AgentUpdateRequest,
    ) -> Result<AgentUpdateReceipt, Fault> {
        request
            .changes
            .validate()
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        let original = self.info(caller)?;
        if original.config_revision != request.expected_config_revision {
            return Err(Fault::new(
                FaultKind::AgentConfigConflict,
                "Agent configuration revision changed",
            ));
        }
        let config = original
            .config
            .apply_patch(&request.changes.config)
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        let name = request.changes.name.unwrap_or(original.name);
        let resources = self.resources_owned()?;
        let candidate = SessionConfig::from_agent(&config, &SessionOverrides::default())
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "configuration validation requires a live Tokio runtime",
            )
        })?;
        // Pure preparation must not pin the caller's async poll. Dropping this
        // wait leaves no accepted mutation; only the owner boundary below may
        // publish the candidate or retain its original receipt.
        runtime
            .spawn_blocking(move || resources.validate(&candidate))
            .await
            .map_err(|_| {
                Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "configuration validation task ended unexpectedly",
                )
            })
            .and_then(|result| result)
            .inspect_err(|fault| {
                self.metadata_refusal_record(Some(request.operation_id.clone()), fault)
                    .emit();
            })?;
        let (receipt, _) = self.accept_prepared_update_candidate(
            caller,
            request.expected_config_revision,
            (config, name),
            Some(request.operation_id.clone()),
            |state| {
                let result = if state.info.metadata_saving {
                    MetadataUpdateResult::Saving {
                        config_revision: state.info.config_revision,
                    }
                } else {
                    MetadataUpdateResult::Committed {
                        config_revision: state.info.config_revision,
                    }
                };
                let receipt = AgentUpdateReceipt {
                    kind: AgentUpdateKind::AgentUpdate,
                    host_id: self.host_id.clone(),
                    host_incarnation: self.incarnation.clone(),
                    agent_id: self.id.clone(),
                    operation_id: request.operation_id,
                    result,
                };
                Self::retain_control(state, Receipt::AgentUpdate(receipt.clone()));
                receipt
            },
        )?;
        Ok(receipt)
    }
}
