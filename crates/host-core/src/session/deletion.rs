use super::*;
use crate::persistence::disk;

impl SessionController {
    /// The direct Agent reserves this child before asking its actual owner to
    /// fence input/control admission. No parent lock is held across this call.
    pub(crate) fn claim_delete_owned(
        &self,
        close: bool,
        operation: &OperationId,
    ) -> Result<(SessionDeleteAccepted, Vec<Record>), Fault> {
        self.claim_delete_state_owned(close, false, Some(operation.clone()))
    }
    /// A cascade already admitted normal close for every explicit child before
    /// its Agent receipt. Claim data deletion without repeating scope cleanup.
    pub(crate) fn claim_cascade_delete_owned(
        &self,
        close: bool,
    ) -> Result<(SessionDeleteAccepted, Vec<Record>), Fault> {
        self.claim_delete_state_owned(close, close, None)
    }
    fn claim_delete_state_owned(
        &self,
        close: bool,
        close_already_claimed: bool,
        operation: Option<OperationId>,
    ) -> Result<(SessionDeleteAccepted, Vec<Record>), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.reclaimed {
            return Err(Fault::new(
                FaultKind::AgentSessionNotFound,
                "Session was reclaimed",
            ));
        }
        if state.deleting || state.releasing {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "Session cleanup already has an owner",
            ));
        }
        if !close && (state.lifecycle != SessionLifecycle::Closed || !Self::work_settled(&state)) {
            return Err(Fault::new(
                FaultKind::SessionDeleteInvalid,
                "default deletion requires a closed, settled Session",
            ));
        }
        let previous = state.lifecycle;
        let cancellable = Self::cancellable(&state);
        state.deleting = true;
        state.deletion = Some(SessionDeletionState {
            close,
            data: DataDeletion::Pending {},
        });
        if close && !close_already_claimed {
            self.fence_delete_close_locked(&mut state);
        }
        debug_assert!(!close || state.lifecycle != SessionLifecycle::Open);
        self.publish(&mut state);
        let records = self.close_records(&state, previous, cancellable, "delete", operation);
        Ok((
            SessionDeleteAccepted {
                lifecycle: match state.lifecycle {
                    SessionLifecycle::Closed => SessionCloseLifecycle::Closed,
                    _ => SessionCloseLifecycle::Closing,
                },
                close,
            },
            records,
        ))
    }
    /// Use normal close admission, including accepted-inbox disposition, before
    /// a queued driver can conclude memory Closed. Scope callbacks are spawned,
    /// not invoked here; Runtime shutdown and result handoff remain async work.
    fn fence_delete_close_locked(&self, state: &mut SessionState) {
        self.start_scope_cleanup_locked(state, true);
        let _ = self.apply_close_locked(state);
    }
    pub(crate) fn fence_delete_close_owned(&self, operation: &OperationId) -> Vec<Record> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let previous = state.lifecycle;
        let cancellable = Self::cancellable(&state);
        self.fence_delete_close_locked(&mut state);
        self.publish(&mut state);
        self.close_records(
            &state,
            previous,
            cancellable,
            "delete",
            Some(operation.clone()),
        )
    }
    pub(crate) fn begin_delete_close_owned(&self) {
        let handle = {
            let state = self.state.lock().expect("Session mutex poisoned");
            debug_assert!(state.lifecycle != SessionLifecycle::Open);
            state
                .active
                .as_ref()
                .and_then(|active| active.handle.clone())
        };
        if let Some(handle) = handle {
            handle.shutdown();
        }
        self.handoff_delegation_owned();
    }
    /// Unlike an unbounded close waiter, deletion reports a real retained save
    /// failure. It does not retry or discard the original required save.
    pub(crate) async fn settle_delete_owned(&self) -> Result<(), Fault> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.state.lock().expect("Session mutex poisoned");
                if let Some(fault) = state
                    .history_work
                    .as_ref()
                    .and_then(|work| work.fault.clone())
                    .or_else(|| {
                        state
                            .record_work
                            .as_ref()
                            .and_then(|work| work.fault.clone())
                    })
                    .or_else(|| state.close_fault.clone())
                    .or_else(|| {
                        state
                            .active
                            .as_ref()
                            .and_then(|active| active.tool_cleanup.fault.clone())
                    })
                {
                    return Err(fault);
                }
                if Self::work_settled(&state) {
                    return Ok(());
                }
            }
            changed.await;
        }
    }
    pub(crate) async fn delete_data_owned(
        self: &Arc<Self>,
        operation: Option<&OperationId>,
    ) -> Result<DataDeletion, Fault> {
        self.settle_delete_owned().await?;
        self.finish_close_owned(operation).await?;
        self.join_close_owned().await?;
        let store = self.store_owned();
        let mode = self.storage_mode();
        if mode == StorageMode::Persistent && store.is_none() {
            return Err(Fault::new(
                FaultKind::SessionDeleteFailed,
                "persistent Session has no retained storage owner",
            ));
        }
        // The real release joins driver/load/save/scope/tool work and releases
        // the history writer; a persisted acknowledgement alone is not enough.
        self.release_resources(true).await?;
        if let Some(store) = store {
            let source = store.clone();
            let result = disk(FaultKind::SessionDeleteFailed, move || {
                source.delete_closed()
            })
            .await;
            if let Err(error) = result {
                // Retain the actual locator and failure after partial removal.
                // Execution stays released; this is not rollback or recreation.
                *self.store.lock().expect("Session store mutex poisoned") = Some(store);
                return Err(error);
            }
        }
        drop(
            self.store
                .lock()
                .expect("Session store mutex poisoned")
                .take(),
        );
        self.scope_root_closed_owned();
        self.reclaim_bodies();
        Ok(match mode {
            StorageMode::Persistent => DataDeletion::Deleted {},
            StorageMode::Memory => DataDeletion::Reclaimed {},
        })
    }
    pub(crate) fn deletion_result_owned(&self, data: DataDeletion) -> SessionDeleteResult {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.deleting = false;
        if let Some(deletion) = &mut state.deletion {
            deletion.data = data.clone();
        }
        self.publish(&mut state);
        SessionDeleteResult {
            lifecycle: state.lifecycle,
            data,
        }
    }
    pub(crate) fn delete_close_failure_owned(&self) -> Option<Fault> {
        let state = self.state.lock().expect("Session mutex poisoned");
        if state.lifecycle == SessionLifecycle::Closed {
            return None;
        }
        match state.deletion.as_ref().map(|deletion| &deletion.data) {
            Some(DataDeletion::Failed { error }) => Some(error.clone()),
            _ => None,
        }
    }
    pub(crate) fn reclaimed_deletion_fact_owned(&self) -> Option<SessionDeleteResult> {
        let state = self.state.lock().expect("Session mutex poisoned");
        (state.reclaimed && state.info.mode == StorageMode::Memory).then_some(SessionDeleteResult {
            lifecycle: state.lifecycle,
            data: DataDeletion::Reclaimed {},
        })
    }
    pub(crate) fn unattempted_deletion_owned(&self) -> SessionDeleteResult {
        let state = self.state.lock().expect("Session mutex poisoned");
        SessionDeleteResult {
            lifecycle: state.lifecycle,
            data: DataDeletion::NotAttempted {},
        }
    }
}
