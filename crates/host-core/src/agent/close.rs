//! Agent-owned Session close orchestration and default-metadata barrier.
use super::*;

impl AgentOwner {
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
                self.remove_retired_session_owned(&child, Some(&operation));
            }
            self.state
                .lock()
                .expect("Agent mutex poisoned")
                .close_running
                .remove(id);
            self.changed.notify_waiters();
            self.try_retire_empty_owned(Some(&operation));
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
                    child.finish_close_owned(Some(&operation)).await?;
                    child.join_close_owned().await?;
                    if child.storage_mode() == StorageMode::Memory {
                        child.retire_closed_owned().await?;
                        owner.remove_retired_session_owned(&child, Some(&operation));
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
}
