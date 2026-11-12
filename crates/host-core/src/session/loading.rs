//! Explicit load, resume, pause and residency transitions under the Session owner.
use super::*;

impl SessionController {
    pub(super) fn start_load_owned(
        self: &Arc<Self>,
        state: &mut SessionState,
        resume: bool,
        operation: OperationId,
    ) -> Result<(), Fault> {
        let load_id = LoadId::new()
            .map_err(|error| Fault::new(FaultKind::SessionInternalUnexpected, error.to_string()))?;
        Self::open(state)?;
        if state.active.is_some()
            || state.preparing
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
            let records = {
                let mut state = owner.state.lock().expect("Session mutex poisoned");
                let previous = state.lifecycle;
                let schedule = diagnostics::Schedule::capture(&state);
                state.preparing = false;
                let result = result.and_then(|history| {
                    if persistent {
                        state.replace_history(&history)?;
                    }
                    Ok(())
                });
                let failed = result.as_ref().err().map(|fault| {
                    diagnostics::history_failed(
                        minicore_host_diagnostics::Context {
                            operation_id: Some(operation.clone()),
                            ..diagnostic::session_context(&owner.identity())
                        },
                        fault,
                        "load",
                        state.controller_revision,
                    )
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
                let mut records = failed
                    .map(|record| {
                        record.published(state.controller_revision, state.runner_revision)
                    })
                    .into_iter()
                    .collect::<Vec<_>>();
                records.extend(owner.closed_record(&state, previous));
                records.extend(owner.scheduling_record(
                    &state,
                    schedule,
                    "load_finished",
                    Some(&operation),
                ));
                records
            };
            diagnostic::emit(records);
            owner.handoff_delegation_owned();
            owner.wake();
        }));
        self.publish(state);
        Ok(())
    }
    pub(super) fn finish_load_resume(&self, state: &mut SessionState) {
        if state.resume_after_load.take() == Some(state.pause_generation)
            && state.lifecycle == SessionLifecycle::Open
            && !state.host_stopping
            && self.access.execution_allowed_owned()
            && state.block_reason.is_none()
        {
            state.paused = false;
            state.failure_paused = false;
            state.unloaded_with_input = false;
        }
    }
    pub(crate) fn control(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: SessionControlRequest,
        kind: SessionControlKind,
    ) -> Result<SessionControlReceipt, Fault> {
        if matches!(kind, SessionControlKind::Load | SessionControlKind::Resume) {
            self.ensure_executor()?;
        }
        let (receipt, record) = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let previous = diagnostics::Schedule::capture(&state);
            match kind {
                SessionControlKind::Pause => {
                    Self::open(&state)?;
                    state.pause_generation =
                        state.pause_generation.checked_add(1).ok_or_else(|| {
                            Fault::new(
                                FaultKind::SessionInternalUnexpected,
                                "pause generation exhausted",
                            )
                        })?;
                    state.paused = true;
                    self.publish(&mut state);
                }
                SessionControlKind::Resume => {
                    if state.history_work.is_some() || state.record_work.is_some() {
                        let resume_scheduling =
                            self.access.execution_allowed_owned() && !state.host_stopping;
                        self.start_save_continuations(&mut state, resume_scheduling)?;
                    } else if self.resume_scope_cleanup(&mut state) {
                        // Explicitly continue the registered cleanup only, even
                        // during force shutdown. This never restarts business work.
                    } else {
                        self.access.ensure_execution_owned()?;
                        Self::open(&state)?;
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
                            // Explicit resume permits one fresh preparation of
                            // queued input; the driver rechecks dependencies
                            // before binding it and retains any new fault.
                            state.block_reason = None;
                        }
                        if state.residency == Residency::Loading {
                            state.resume_after_load = Some(state.pause_generation);
                            self.publish(&mut state);
                        } else if state.info.mode == StorageMode::Persistent
                            && (!state.history_loaded
                                || state.block_reason.as_ref().is_some_and(|fault| {
                                    fault.kind() == Some(FaultKind::SessionHistoryLoadFailed)
                                }))
                        {
                            self.start_load_owned(&mut state, true, request.operation_id.clone())?;
                        } else {
                            if let Some(fault) = &state.block_reason {
                                return Err(fault.clone());
                            }
                            state.paused = false;
                            state.failure_paused = false;
                            state.unloaded_with_input = false;
                            self.publish(&mut state);
                        }
                    }
                }
                SessionControlKind::Load => {
                    self.access.ensure_execution_owned()?;
                    self.start_load_owned(&mut state, false, request.operation_id.clone())?;
                }
                SessionControlKind::Unload => {
                    Self::open(&state)?;
                    if state.active.is_some()
                        || state.compaction.is_some()
                        || state.history_work.is_some()
                        || state.preparing
                        || state.record_work.as_ref().is_some_and(|work| work.running)
                    {
                        return Err(Fault::new(
                            FaultKind::SessionBusy,
                            "Session still owns execution or save work",
                        ));
                    }
                    if !state.next_step.is_empty() || !state.next_turn.is_empty() {
                        state.unloaded_with_input = true;
                    }
                    state.residency = Residency::Unloaded;
                    state.load_id = None;
                    if state.info.mode == StorageMode::Persistent {
                        state.history_loaded = false;
                    }
                    self.publish(&mut state);
                }
            }
            let record = self.scheduling_record(
                &state,
                previous,
                match kind {
                    SessionControlKind::Pause => "pause",
                    SessionControlKind::Resume => "resume",
                    SessionControlKind::Load => "load",
                    SessionControlKind::Unload => "unload",
                },
                Some(&request.operation_id),
            );
            let receipt = SessionControlReceipt {
                kind,
                host_id: self.host_id.clone(),
                host_incarnation: self.incarnation.clone(),
                operation_id: request.operation_id,
                agent_id: self.agent_id.clone(),
                session_id: self.id.clone(),
                result: Self::control_facts(&state),
            };
            Self::retain_control(&mut state, Receipt::SessionControl(receipt.clone()));
            Ok((receipt, record))
        })?;
        diagnostic::emit(record);
        if kind == SessionControlKind::Resume {
            self.wake();
        }
        Ok(receipt)
    }
    // Legacy test convenience calls use the same acceptance and state mutation.
    #[cfg(test)]
    pub(crate) fn pause(self: &Arc<Self>, caller: &CallerContext) -> Result<(), Fault> {
        self.control(
            caller,
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            SessionControlKind::Pause,
        )
        .map(|_| ())
    }
    #[cfg(test)]
    pub(crate) fn resume(self: &Arc<Self>, caller: &CallerContext) -> Result<(), Fault> {
        self.control(
            caller,
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            SessionControlKind::Resume,
        )
        .map(|_| ())
    }
}

impl SessionState {
    pub(super) fn replace_history(&mut self, entries: &[HistoryEntry]) -> Result<(), Fault> {
        let mut raw = Vec::new();
        let mut input_provenance = input_provenance::InputProvenance::default();
        let mut projection: Option<minicore_host_storage::CompactionProjection> = None;
        for (position, entry) in entries.iter().enumerate() {
            if let HistoryEntry::Compaction {
                source_position,
                covered_items,
                summary,
            } = entry
            {
                let candidate = minicore_host_storage::CompactionProjection::new(
                    *source_position,
                    *covered_items,
                    summary.clone(),
                );
                candidate
                    .validate(
                        &raw,
                        position as u64,
                        projection.as_ref().map_or(0, |value| value.covered_items),
                    )
                    .map_err(|error| {
                        Fault::new(FaultKind::SessionHistoryLoadFailed, error.to_string())
                    })?;
                projection = Some(candidate);
            } else if let Some(item) = entry.runtime_item() {
                input_provenance.record(raw.len(), entry);
                raw.push(item);
            }
        }
        self.history = Arc::new(raw);
        self.input_provenance = input_provenance;
        self.history_position = entries.len() as u64;
        self.projection = projection;
        self.readable_history = Some(
            entries
                .iter()
                .filter_map(HistoryItemData::from_entry)
                .collect(),
        );
        Ok(())
    }
}
