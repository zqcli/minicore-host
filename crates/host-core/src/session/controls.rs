use super::*;

impl SessionController {
    pub(crate) fn remove(
        &self,
        caller: &CallerContext,
        message_id: &MessageId,
    ) -> Result<(), Fault> {
        let operation_id = OperationId::new()
            .map_err(|error| Fault::new(FaultKind::SessionInternalUnexpected, error.to_string()))?;
        self.remove_control(
            caller,
            InputRemoveRequest {
                operation_id,
                message_id: message_id.clone(),
                expected_controller_revision: None,
            },
        )
        .map(|_| ())
    }
    pub(crate) fn input_list(&self, caller: &CallerContext) -> Result<InputList, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            let entry = |queued: &QueuedInput| QueuedInputRecord {
                input: queued.message.clone(),
                source: queued.source.clone(),
                queue: queued.queue,
                order: queued.order,
            };
            Ok(InputList {
                host_id: self.host_id.clone(),
                host_incarnation: self.incarnation.clone(),
                agent_id: self.agent_id.clone(),
                session_id: self.id.clone(),
                controller_revision: state.controller_revision,
                next_step: state.next_step.iter().map(entry).collect(),
                next_turn: state.next_turn.iter().map(entry).collect(),
            })
        })
    }
    pub(crate) fn remove_control(
        &self,
        caller: &CallerContext,
        request: InputRemoveRequest,
    ) -> Result<InputRemoveReceipt, Fault> {
        let message_id = &request.message_id;
        let result = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::available(&state)?;
            let position = state
                .next_step
                .iter()
                .position(|input| &input.message.message_id == message_id)
                .map(|index| (InputQueue::NextStep, index))
                .or_else(|| {
                    state
                        .next_turn
                        .iter()
                        .position(|input| &input.message.message_id == message_id)
                        .map(|index| (InputQueue::NextTurn, index))
                });
            let Some((queue, index)) = position else {
                let bound = state.messages.iter().any(|input| {
                    &input.input.message_id == message_id
                        && matches!(input.binding, MessageBinding::Bound { .. })
                });
                return Err(Fault::new(
                    if bound {
                        FaultKind::SessionInputAlreadyBound
                    } else {
                        FaultKind::SessionInputNotFound
                    },
                    "input is not an unbound queue entry",
                ));
            };
            if request
                .expected_controller_revision
                .is_some_and(|expected| expected != state.controller_revision)
            {
                return Err(Fault::new(
                    FaultKind::SessionInputInvalid,
                    "input removal Controller revision changed",
                ));
            }
            let removed = match queue {
                InputQueue::NextStep => state.next_step.remove(index),
                InputQueue::NextTurn => state.next_turn.remove(index),
            }
            .expect("located input");
            mark_message(&mut state, &removed, MessageBinding::Removed);
            self.publish(&mut state);
            let receipt = InputRemoveReceipt {
                kind: InputRemoveKind::InputRemove,
                host_id: self.host_id.clone(),
                host_incarnation: self.incarnation.clone(),
                operation_id: request.operation_id,
                agent_id: self.agent_id.clone(),
                session_id: self.id.clone(),
                message_id: request.message_id.clone(),
                result: InputRemoveResult {
                    queue,
                    order: removed.order,
                    controller_revision: state.controller_revision,
                },
            };
            Self::retain_control(&mut state, Receipt::InputRemove(receipt.clone()));
            Ok(receipt)
        });
        if result.is_ok() {
            self.handoff_delegation_owned();
        }
        result
    }

    /// The same projection supplies state snapshots and receipts. The caller
    /// holds this Session's state lock, so these are formation-time facts.
    pub(super) fn control_facts(state: &SessionState) -> SessionControlFacts {
        let pause_reasons = [
            state.paused.then_some(PauseReason::Explicit),
            state
                .failure_paused
                .then_some(PauseReason::ExecutionFailure),
            state
                .unloaded_with_input
                .then_some(PauseReason::UnloadedWithInput),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        SessionControlFacts {
            scheduling: if pause_reasons.is_empty() {
                Scheduling::Auto
            } else {
                Scheduling::Paused
            },
            pause_reasons,
            block_reason: state.block_reason.as_ref().map(|fault| BlockReason {
                message: fault.message().to_owned(),
                resolution: match fault.kind() {
                    Some(FaultKind::SessionHistoryWriteFailed) => {
                        "Explicitly resume to confirm the original save before continuing"
                    }
                    Some(FaultKind::SessionHistoryLoadFailed) => {
                        "Restore valid history, then explicitly load or resume to validate it"
                    }
                    Some(FaultKind::SessionResourceMissing) => {
                        "Correct the Session resource configuration before resuming"
                    }
                    _ => "Resolve the reported dependency or execution fault before resuming",
                }
                .into(),
                fault: Some(fault.clone()),
            }),
            residency: state.residency,
            load_id: state.load_id.clone(),
            controller_revision: state.controller_revision,
        }
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
    pub(super) fn retain_control(state: &mut SessionState, receipt: Receipt) {
        state
            .controls
            .entry(receipt.operation_id().clone())
            .or_default()
            .push(receipt);
    }
    pub(crate) fn control_receipts_owned(&self, operation: &OperationId) -> Vec<Receipt> {
        let state = self.state.lock().expect("Session mutex poisoned");
        if state.reclaimed {
            return Vec::new();
        }
        state.controls.get(operation).cloned().unwrap_or_default()
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
        let receipt = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
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
                            self.start_load_owned(&mut state, true)?;
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
                    self.start_load_owned(&mut state, false)?;
                }
                SessionControlKind::Unload => {
                    Self::open(&state)?;
                    if state.active.is_some()
                        || state.compaction.is_some()
                        || state.history_work.is_some()
                        || state.preparing
                        || state.saving
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
            Ok(receipt)
        })?;
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
    pub(crate) fn cancel(
        &self,
        caller: &CallerContext,
        target: Option<&TurnId>,
        keep_inbox: bool,
        operation: Option<OperationId>,
    ) -> Result<(CancelResult, Option<Receipt>), Fault> {
        let (result, receipt, handle) = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::available(&state)?;
            if let Some(id) = target
                && state
                    .active
                    .as_ref()
                    .is_none_or(|active| &active.info.id != id)
                && !state.completed.contains_key(id)
            {
                return Err(Fault::new(
                    FaultKind::SessionTurnNotFound,
                    "Turn is not held by this Session",
                ));
            }
            let mut handle = None;
            let status = match state.active.as_mut() {
                Some(active) if target.is_none_or(|id| id == &active.info.id) => {
                    if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                        active._activation.expire();
                        active.info.state = TurnState::Cancelling;
                        handle = active.handle.clone();
                        CancelStatus::Claimed
                    } else {
                        CancelStatus::AlreadyEnding
                    }
                }
                _ => CancelStatus::NotActive,
            };
            if status == CancelStatus::Claimed {
                Self::cancel_compaction_locked(&mut state);
            }
            let clear = !keep_inbox
                && (status == CancelStatus::Claimed
                    || (target.is_none() && status == CancelStatus::NotActive));
            let cleared = if clear {
                clear_inbox(&mut state)
            } else {
                Vec::new()
            };
            self.publish(&mut state);
            let turn = match target {
                Some(id) => state
                    .active
                    .as_ref()
                    .filter(|active| &active.info.id == id)
                    .map(|active| &active.info)
                    .or_else(|| state.completed.get(id).map(|result| &result.info)),
                None => state.active.as_ref().map(|active| &active.info),
            };
            let turn = turn.map(|turn| (turn.id.clone(), turn.state));
            let receipt = operation.map(|operation_id| {
                let receipt = if let Some(id) = target {
                    Receipt::TurnCancel(TurnCancelReceipt {
                        kind: TurnCancelKind::TurnCancel,
                        host_id: self.host_id.clone(),
                        host_incarnation: self.incarnation.clone(),
                        operation_id,
                        agent_id: self.agent_id.clone(),
                        session_id: self.id.clone(),
                        turn_id: id.clone(),
                        result: TurnCancelResult {
                            cancel_claimed: status == CancelStatus::Claimed,
                            turn_state: turn.as_ref().expect("validated Turn").1,
                            controller_revision: state.controller_revision,
                        },
                    })
                } else {
                    Receipt::SessionCancel(SessionCancelReceipt {
                        kind: SessionCancelKind::SessionCancel,
                        host_id: self.host_id.clone(),
                        host_incarnation: self.incarnation.clone(),
                        operation_id,
                        agent_id: self.agent_id.clone(),
                        session_id: self.id.clone(),
                        turn_id: turn.as_ref().map(|turn| turn.0.clone()),
                        result: SessionCancelResult {
                            cancel_claimed: status == CancelStatus::Claimed,
                            turn_state: turn.as_ref().map(|turn| turn.1),
                            controller_revision: state.controller_revision,
                        },
                    })
                };
                Self::retain_control(&mut state, receipt.clone());
                receipt
            });
            Ok((CancelResult { status, cleared }, receipt, handle))
        })?;
        if let Some(handle) = handle {
            handle.cancel();
        }
        self.handoff_delegation_owned();
        Ok((result, receipt))
    }
    pub(super) fn apply_close_locked(&self, state: &mut SessionState) -> Option<LoopHandle> {
        if state.lifecycle == SessionLifecycle::Closed {
            None
        } else {
            if state.lifecycle == SessionLifecycle::Open {
                state.end_reason = Some("explicit close".into());
            }
            state.lifecycle = SessionLifecycle::Closing;
            Self::cancel_compaction_locked(state);
            clear_inbox(state);
            let handle = state.active.as_mut().and_then(|active| {
                if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                    active._activation.expire();
                    active.info.state = TurnState::Cancelling;
                }
                active.handle.clone()
            });
            Self::finish_memory_close(state);
            self.publish(state);
            handle
        }
    }
    pub(super) fn close_owned(
        &self,
        operation: OperationId,
    ) -> Result<(SessionCloseReceipt, Option<LoopHandle>), Fault> {
        self.start_scope_cleanup_owned();
        let mut state = self.state.lock().expect("Session mutex poisoned");
        Self::available(&state)?;
        let handle = self.apply_close_locked(&mut state);
        let receipt = SessionCloseReceipt {
            kind: SessionCloseKind::SessionClose,
            host_id: self.host_id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id: operation,
            agent_id: self.agent_id.clone(),
            session_id: self.id.clone(),
            result: SessionCloseResult {
                lifecycle: match state.lifecycle {
                    SessionLifecycle::Closed => SessionCloseLifecycle::Closed,
                    _ => SessionCloseLifecycle::Closing,
                },
                controller_revision: state.controller_revision,
            },
        };
        Self::retain_control(&mut state, Receipt::SessionClose(receipt.clone()));
        Ok((receipt, handle))
    }
}
