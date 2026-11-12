//! Close admission and completion; persistent record commits remain in metadata.
use super::collaboration::DelegationSlot;
use super::*;

impl SessionController {
    pub(super) fn apply_close_locked(&self, state: &mut SessionState) -> Option<CancellationToken> {
        if state.lifecycle == SessionLifecycle::Closed {
            None
        } else {
            if state.lifecycle == SessionLifecycle::Open {
                state.end_reason = Some("explicit close".into());
            }
            state.lifecycle = SessionLifecycle::Closing;
            Self::cancel_compaction_locked(state);
            clear_inbox(state);
            let handle = state.active.as_mut().map(|active| {
                if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                    active._activation.expire();
                    active.info.state = TurnState::Cancelling;
                }
                active.cancellation.clone()
            });
            Self::finish_memory_close(state);
            self.publish(state);
            handle
        }
    }
    pub(super) fn close_owned(
        &self,
        operation: OperationId,
    ) -> Result<(SessionCloseReceipt, Option<CancellationToken>, Vec<Record>), Fault> {
        self.start_scope_cleanup_owned();
        let mut state = self.state.lock().expect("Session mutex poisoned");
        Self::available(&state)?;
        let previous = state.lifecycle;
        let cancellable = Self::cancellable(&state);
        let handle = self.apply_close_locked(&mut state);
        let records = self.close_records(
            &state,
            previous,
            cancellable,
            "close",
            Some(operation.clone()),
        );
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
        Ok((receipt, handle, records))
    }
    pub(crate) async fn join_close_owned(&self) -> Result<(), Fault> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.state.lock().expect("Session mutex poisoned");
                if let Some(fault) = &state.close_fault {
                    return Err(fault.clone());
                }
                if state.released || state.reclaimed {
                    return Ok(());
                }
                if state.lifecycle == SessionLifecycle::Closed
                    && !state.driver_running
                    && !state.handoff_pending
                    && !state.scope_cleanup_pending
                    && state.history_work.is_none()
                    && state.record_work.is_none()
                {
                    return Ok(());
                }
                if let Some(fault) = state
                    .history_work
                    .as_ref()
                    .and_then(|work| work.fault.clone())
                {
                    return Err(fault);
                }
            }
            changed.await;
        }
    }
    pub(super) fn finish_memory_close(state: &mut SessionState) {
        if state.lifecycle == SessionLifecycle::Closing
            && state.info.mode == StorageMode::Memory
            && state.active.is_none()
            && state.compaction.is_none()
            && !state.preparing
            && state.history_work.is_none()
            && state.record_work.is_none()
            && !state.handoff_pending
            && !state.scope_cleanup_pending
            && state.close_fault.is_none()
            && !state
                .delegation
                .as_ref()
                .is_some_and(DelegationSlot::needs_handoff)
            && state
                .root_scope
                .as_ref()
                .is_none_or(|scope| scope.pending.load(Ordering::SeqCst) == 0)
        {
            state.lifecycle = SessionLifecycle::Closed;
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
    }
    pub(crate) fn begin_close_owned(
        &self,
        operation: OperationId,
    ) -> Result<(SessionCloseReceipt, Vec<Record>), Fault> {
        let (receipt, handle, records) = self.close_owned(operation)?;
        if let Some(handle) = handle {
            handle.cancel();
        }
        Ok((receipt, records))
    }
    pub(crate) fn set_close_fault(&self, fault: Fault) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.close_fault = Some(fault);
        self.publish(&mut state);
    }
}
