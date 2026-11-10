use super::*;

impl AgentOwner {
    /// Accept an actual close worker before awaiting anything. Dropping any
    /// caller's permit wait cannot discard the accepted close/save/retirement.
    pub(crate) async fn close_internal_owned(
        self: &Arc<Self>,
        id: &SessionId,
        operation: OperationId,
    ) -> Result<(), Fault> {
        let child = self.session(id)?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "close requires a live Tokio runtime",
            )
        })?;
        let mut diagnostics = Vec::new();
        let accepted = self.access.accept_internal_owned(|| {
            let first = {
                let mut state = self.state.lock().expect("Agent mutex poisoned");
                if state.deleting_sessions.contains(id) {
                    // Existing scope cleanup waits for the already accepted
                    // delete's normal close; it must not start a second worker.
                    return Ok(());
                }
                state.close_running.insert(id.clone())
            };
            let records = match child.begin_close_owned(operation) {
                Ok((_, records)) => records,
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
            diagnostics.extend(records);
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            state.closing.insert(id.clone());
            if first {
                let owner = Arc::clone(self);
                let target = Arc::clone(&child);
                let id = id.clone();
                let key = id.clone();
                let task = runtime.spawn(async move {
                    let result = async {
                        target.handoff_delegation_owned();
                        if owner.store().is_some() {
                            owner.clear_default_before_close(&id).await?;
                        } else {
                            owner.clear_default_if(&id);
                        }
                        target.finish_close_owned().await?;
                        target.join_close_owned().await?;
                        if target.storage_mode() == StorageMode::Memory {
                            target.retire_closed_owned().await?;
                            owner.remove_retired_session_owned(&target);
                        }
                        Ok::<_, Fault>(())
                    }
                    .await;
                    if let Err(fault) = result {
                        target.set_close_fault(fault);
                    }
                    {
                        let mut state = owner.state.lock().expect("Agent mutex poisoned");
                        state.close_running.remove(&id);
                        state.close_tasks.remove(&id);
                    }
                    owner.changed.notify_waiters();
                    owner.try_retire_empty_owned();
                });
                state.close_tasks.insert(key, task);
            }
            Ok(())
        });
        crate::diagnostics::emit(diagnostics);
        accepted?;
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let pending = {
                let state = self.state.lock().expect("Agent mutex poisoned");
                state.close_running.contains(id) || state.deleting_sessions.contains(id)
            };
            if !pending {
                if let Some(error) = child.delete_close_failure_owned() {
                    return Err(error);
                }
                return child.join_close_owned().await;
            }
            changed.await;
        }
    }
}
