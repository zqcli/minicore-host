use super::*;
use crate::persistence::disk;
use futures_util::FutureExt;
use std::collections::BTreeSet;
use std::panic::AssertUnwindSafe;

impl AgentOwner {
    pub(crate) fn delete_session(
        self: &Arc<Self>,
        caller: &CallerContext,
        id: &SessionId,
        request: SessionDeleteRequest,
    ) -> Result<SessionDeleteReceipt, Fault> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "deletion requires a live Tokio runtime",
            )
        })?;
        let operation = request.operation_id.clone();
        let (receipt, diagnostics) = self.access.accept_execution(caller, || {
            let child = {
                let mut state = self.state.lock().expect("Agent mutex poisoned");
                if state.deleting
                    || matches!(
                        state.info.lifecycle,
                        AgentLifecycle::Retiring | AgentLifecycle::Retired
                    )
                {
                    return Err(Fault::new(
                        FaultKind::AgentLifecycleUnavailable,
                        "Agent retirement already owns its children",
                    ));
                }
                if state.close_running.contains(id) || state.deleting_sessions.contains(id) {
                    return Err(Fault::new(
                        FaultKind::SessionBusy,
                        "Session cleanup already has a direct owner",
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
                state.deleting_sessions.insert(id.clone());
                child
            };
            let (accepted, diagnostics) =
                match child.claim_delete_owned(request.close, &request.operation_id) {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        self.state
                            .lock()
                            .expect("Agent mutex poisoned")
                            .deleting_sessions
                            .remove(id);
                        return Err(error);
                    }
                };
            let receipt = SessionDeleteReceipt {
                kind: SessionDeleteKind::SessionDelete,
                host_id: self.host_id.clone(),
                host_incarnation: self.incarnation.clone(),
                agent_id: self.id.clone(),
                session_id: id.clone(),
                operation_id: request.operation_id,
                result: accepted,
            };
            let owner = Arc::clone(self);
            let task_id = id.clone();
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            state.closing.insert(id.clone());
            Self::retain_control(&mut state, Receipt::SessionDelete(receipt.clone()));
            let task = runtime.spawn(
                async move {
                    let data = AssertUnwindSafe(owner.delete_session_work(
                        &child,
                        request.close,
                        &operation,
                    ))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Err(Fault::new(
                            FaultKind::SessionDeleteFailed,
                            "Session deletion worker panicked",
                        ))
                    })
                    .unwrap_or_else(|error| DataDeletion::Failed { error });
                    child.deletion_result_owned(data.clone());
                    if data.removed() {
                        owner.remove_retired_session_owned(&child, Some(&operation));
                    }
                    {
                        let mut state = owner.state.lock().expect("Agent mutex poisoned");
                        state.deleting_sessions.remove(&task_id);
                        state.delete_tasks.remove(&task_id);
                    }
                    owner.changed.notify_waiters();
                    owner.try_retire_empty_owned(Some(&operation));
                }
                .with_current_subscriber(),
            );
            state.delete_tasks.insert(id.clone(), task);
            Ok((receipt, diagnostics))
        })?;
        crate::diagnostics::emit(diagnostics);
        Ok(receipt)
    }
    async fn delete_session_work(
        self: &Arc<Self>,
        child: &Arc<SessionController>,
        close: bool,
        operation: &OperationId,
    ) -> Result<DataDeletion, Fault> {
        if close {
            child.begin_delete_close_owned();
        }
        if self.store().is_some() {
            self.clear_default_before_close(&child.id, operation)
                .await?;
        } else {
            self.clear_default_if(&child.id);
        }
        child.delete_data_owned(Some(operation)).await
    }

    /// This is the Agent admission boundary, shared with create/start/default
    /// and metadata work. Neither directory nesting nor a late child may extend
    /// the caller's explicit set.
    pub(crate) fn claim_agent_delete_owned(
        &self,
        scope: &AgentDeleteScope,
        operation: &OperationId,
    ) -> Result<(Vec<Arc<SessionController>>, Vec<crate::diagnostics::Record>), Fault> {
        scope
            .validate()
            .map_err(|error| Fault::new(FaultKind::AgentDeleteInvalid, error.to_string()))?;
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        Self::allow_configuration_update(&state)?;
        if state.deleting
            || !state.deleting_sessions.is_empty()
            || !state.close_running.is_empty()
            || state.creating != 0
            || !state.creation_committed
            || state.record_work.is_some()
            || state.retirement_joining
            || state.resources_released
        {
            return Err(Fault::new(
                FaultKind::AgentDeleteInvalid,
                "Agent still owns creation, save, close, deletion or retirement work",
            ));
        }
        if !state.load_failures.is_empty() {
            return Err(Fault::new(
                FaultKind::AgentDeleteInvalid,
                "unavailable retained Sessions cannot be silently omitted from deletion",
            ));
        }
        match scope {
            AgentDeleteScope::Empty {} => {
                if state.info.lifecycle != AgentLifecycle::Stopped || !state.sessions.is_empty() {
                    return Err(Fault::new(
                        FaultKind::AgentDeleteInvalid,
                        "default Agent deletion requires stopped with no retained Sessions",
                    ));
                }
            }
            AgentDeleteScope::Sessions { session_ids, close } => {
                if session_ids.iter().collect::<BTreeSet<_>>()
                    != state.sessions.keys().collect::<BTreeSet<_>>()
                {
                    return Err(Fault::new(
                        FaultKind::AgentDeleteInvalid,
                        "cascade scope must name exactly the retained direct Sessions",
                    ));
                }
                if !matches!(
                    state.info.lifecycle,
                    AgentLifecycle::Active | AgentLifecycle::Stopped | AgentLifecycle::Retiring
                ) || (!close
                    && state.info.lifecycle != AgentLifecycle::Stopped
                    && state.info.lifecycle != AgentLifecycle::Retiring)
                {
                    return Err(Fault::new(
                        FaultKind::AgentDeleteInvalid,
                        "cascade without close requires an already stopped Agent",
                    ));
                }
            }
        }
        let diagnostic = (state.info.lifecycle != AgentLifecycle::Retiring).then(|| {
            self.lifecycle_record(
                "agent.retiring",
                "retiring",
                state.info.mode,
                Some(operation.clone()),
            )
        });
        state.deleting = true;
        state.info.lifecycle = AgentLifecycle::Retiring;
        self.enabled.store(false, Ordering::SeqCst);
        state.info.deletion = Some(AgentDeletionState {
            scope: scope.clone(),
            sessions: Vec::new(),
            data: DataDeletion::Pending {},
        });
        let children = state.sessions.values().cloned().collect::<Vec<_>>();
        drop(state);
        // The shared admission boundary is still held, but no Agent state lock
        // crosses a child call. A Retiring receipt must not leave old Turn
        // capabilities or descendant scope admission live until a worker runs.
        let mut diagnostics = diagnostic.into_iter().collect::<Vec<_>>();
        if matches!(scope, AgentDeleteScope::Sessions { close: true, .. }) {
            for child in &children {
                diagnostics.extend(child.fence_delete_close_owned(operation));
            }
        }
        self.changed.notify_waiters();
        Ok((children, diagnostics))
    }
    pub(crate) async fn delete_agent_owned(
        self: &Arc<Self>,
        scope: AgentDeleteScope,
        children: Vec<Arc<SessionController>>,
        operation: OperationId,
    ) {
        let mode = self.state.lock().expect("Agent mutex poisoned").info.mode;
        let result = AssertUnwindSafe(self.delete_agent_work(&scope, &children, &operation, mode))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| {
                let fault = Fault::new(
                    FaultKind::AgentDeleteFailed,
                    "Agent deletion worker panicked",
                );
                self.resource_release_failure_record(
                    "delete_worker",
                    mode,
                    Some(&operation),
                    &fault,
                )
                .emit();
                Err(fault)
            });
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        state.deleting = false;
        let diagnostic = result.as_ref().ok().map(|_| {
            self.lifecycle_record(
                "agent.retired",
                "retired",
                state.info.mode,
                Some(operation.clone()),
            )
        });
        if let Err(error) = result {
            state
                .info
                .deletion
                .as_mut()
                .expect("accepted deletion")
                .data = DataDeletion::Failed { error };
        }
        self.changed.notify_waiters();
        drop(state);
        crate::diagnostics::emit(diagnostic);
    }
    async fn delete_agent_work(
        self: &Arc<Self>,
        scope: &AgentDeleteScope,
        children: &[Arc<SessionController>],
        operation: &OperationId,
        mode: StorageMode,
    ) -> Result<(), Fault> {
        let close = matches!(scope, AgentDeleteScope::Sessions { close: true, .. });
        for child in children {
            if close {
                child.begin_delete_close_owned();
            } else {
                child.set_agent_enabled(false, Some(operation));
            }
        }
        let initial = children
            .iter()
            .map(|child| OwnedSessionDeleteResult {
                session_id: child.id.clone(),
                result: child.unattempted_deletion_owned(),
            })
            .collect();
        self.state
            .lock()
            .expect("Agent mutex poisoned")
            .info
            .deletion
            .as_mut()
            .expect("accepted deletion")
            .sessions = initial;
        for child in children {
            // Existing scope-close work remains with its actual owner. Reserve
            // the next child under the same Agent boundary before calling it.
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                let reserved = {
                    let mut state = self.state.lock().expect("Agent mutex poisoned");
                    if state.close_running.contains(&child.id) {
                        false
                    } else {
                        state.deleting_sessions.insert(child.id.clone());
                        true
                    }
                };
                if reserved {
                    break;
                }
                changed.await;
            }
            // Each child has its own admission boundary, even in a cascade.
            let result =
                match child.claim_cascade_delete_owned(close) {
                    Ok((_, diagnostics)) => {
                        crate::diagnostics::emit(diagnostics);
                        let current = child.unattempted_deletion_owned();
                        {
                            let mut state = self.state.lock().expect("Agent mutex poisoned");
                            let progress = state.info.deletion.as_mut().expect("accepted deletion");
                            let entry = progress
                                .sessions
                                .iter_mut()
                                .find(|entry| entry.session_id == child.id)
                                .expect("explicit child");
                            entry.result = SessionDeleteResult {
                                data: DataDeletion::Pending {},
                                ..current
                            };
                        }
                        self.changed.notify_waiters();
                        let data = self
                            .delete_session_work(child, close, operation)
                            .await
                            .unwrap_or_else(|error| DataDeletion::Failed { error });
                        child.deletion_result_owned(data)
                    }
                    Err(error) => child.reclaimed_deletion_fact_owned().unwrap_or_else(|| {
                        SessionDeleteResult {
                            data: DataDeletion::Failed { error },
                            ..child.unattempted_deletion_owned()
                        }
                    }),
                };
            if result.data.removed() {
                self.remove_retired_session_owned(child, Some(operation));
            }
            let failed = match &result.data {
                DataDeletion::Failed { error } => Some(error.clone()),
                _ => None,
            };
            {
                let mut state = self.state.lock().expect("Agent mutex poisoned");
                state.deleting_sessions.remove(&child.id);
                let progress = state.info.deletion.as_mut().expect("accepted deletion");
                let entry = progress
                    .sessions
                    .iter_mut()
                    .find(|entry| entry.session_id == child.id)
                    .expect("explicit child");
                entry.result = result;
            }
            self.changed.notify_waiters();
            if let Some(error) = failed {
                return Err(error);
            }
        }
        let tasks = {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if !state.sessions.is_empty()
                || state.record_work.is_some()
                || state.creating != 0
                || !state.close_running.is_empty()
            {
                return Err(Fault::new(
                    FaultKind::AgentDeleteFailed,
                    "Agent still retains owned work or children",
                ));
            }
            let mut tasks = std::mem::take(&mut state.create_tasks)
                .into_values()
                .collect::<Vec<_>>();
            tasks.extend(std::mem::take(&mut state.close_tasks).into_values());
            tasks.extend(std::mem::take(&mut state.delete_tasks).into_values());
            tasks.extend(state.stop_task.take());
            tasks.extend(state.record_task.take());
            tasks
        };
        for task in tasks {
            task.await
                .map_err(|_| {
                    Fault::new(
                        FaultKind::AgentDeleteFailed,
                        "Agent worker did not join cleanly",
                    )
                })
                .inspect_err(|fault| {
                    self.resource_release_failure_record(
                        "delete_join",
                        mode,
                        Some(operation),
                        fault,
                    )
                    .emit()
                })?;
        }
        let store = self.store();
        let data = if let Some(store) = store {
            disk(FaultKind::AgentDeleteFailed, move || store.delete_empty())
                .await
                .inspect_err(|fault| {
                    self.resource_release_failure_record(
                        "agent_directory_delete",
                        mode,
                        Some(operation),
                        fault,
                    )
                    .emit()
                })?;
            DataDeletion::Deleted {}
        } else {
            DataDeletion::Reclaimed {}
        };
        drop(
            self.store
                .lock()
                .expect("Agent store mutex poisoned")
                .take(),
        );
        drop(
            self.root_lease
                .lock()
                .expect("Agent lease mutex poisoned")
                .take(),
        );
        drop(
            self.resources
                .lock()
                .expect("Agent resources mutex poisoned")
                .take(),
        );
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        state.resources_released = true;
        state.info.lifecycle = AgentLifecycle::Retired;
        state.info.default_session_id = None;
        state
            .info
            .deletion
            .as_mut()
            .expect("accepted deletion")
            .data = data;
        state.controls.clear();
        state.creations.clear();
        state.public_creations.clear();
        Ok(())
    }
}
