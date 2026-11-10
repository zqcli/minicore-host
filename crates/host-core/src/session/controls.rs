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
    pub(crate) fn cancel(
        &self,
        caller: &CallerContext,
        target: Option<&TurnId>,
        keep_inbox: bool,
        operation: Option<OperationId>,
    ) -> Result<(CancelResult, Option<Receipt>), Fault> {
        let (result, receipt, handle, record) = self.access.accept(caller, || {
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
            let record = self.cancel_claim_record(
                &state,
                status == CancelStatus::Claimed,
                "cancel",
                operation.clone(),
                true,
            );
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
            Ok((CancelResult { status, cleared }, receipt, handle, record))
        })?;
        diagnostic::emit(record);
        if let Some(handle) = handle {
            handle.cancel();
        }
        self.handoff_delegation_owned();
        Ok((result, receipt))
    }
}

impl SessionController {
    pub(crate) fn set_agent_enabled(
        self: &Arc<Self>,
        enabled: bool,
        operation: Option<&OperationId>,
    ) {
        let (handle, record) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.releasing || state.released || state.reclaimed {
                return;
            }
            let newly_claimed = !enabled && Self::cancellable(&state);
            if !enabled {
                Self::cancel_compaction_locked(&mut state);
            }
            let handle = if !enabled {
                state.active.as_mut().and_then(|active| {
                    if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                        active._activation.expire();
                        active.info.state = TurnState::Cancelling;
                        active.handle.clone()
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            self.publish(&mut state);
            let record = self.cancel_claim_record(
                &state,
                newly_claimed,
                "agent_disabled",
                operation.cloned(),
                true,
            );
            (handle, record)
        };
        diagnostic::emit(record);
        if let Some(handle) = handle {
            handle.cancel();
        }
        if enabled {
            self.wake();
        }
    }
}
