use super::*;

pub(crate) struct AgentShutdownStatus {
    pub(crate) drained: bool,
    pub(crate) quiescent: bool,
    pub(crate) fault: Option<Fault>,
}

/// A failed creation save does not establish that its original range was never
/// written. Until this stage has a creation reconciliation owner, retain that
/// fault and the directory rather than claiming clean shutdown.
pub(crate) fn creation_save_fault(step: &CreateStep) -> Option<Fault> {
    match step {
        CreateStep::Failed { error }
            if matches!(
                error.kind(),
                Some(FaultKind::HostStorageUnavailable | FaultKind::SessionHistoryWriteFailed)
            ) =>
        {
            Some(error.clone())
        }
        _ => None,
    }
}

impl AgentOwner {
    pub(crate) fn begin_host_shutdown_owned(&self, force: bool, operation: Option<&OperationId>) {
        let (children, diagnostic) = {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            let strategy = if force {
                HostShutdownStrategy::Force
            } else {
                HostShutdownStrategy::Graceful
            };
            if matches!(
                state.host_shutdown_strategy,
                Some(HostShutdownStrategy::Force)
            ) || state.host_shutdown_strategy == Some(strategy)
            {
                return;
            }
            state.host_shutdown_strategy = Some(strategy);
            if force {
                self.enabled.store(false, Ordering::SeqCst);
                if !state.deleting
                    && !matches!(
                        state.info.lifecycle,
                        AgentLifecycle::Retiring | AgentLifecycle::Retired
                    )
                {
                    state.info.lifecycle = AgentLifecycle::Stopping;
                }
            }
            (
                state.sessions.values().cloned().collect::<Vec<_>>(),
                self.lifecycle_record(
                    "agent.host_shutdown_started",
                    if force { "stopping" } else { "draining" },
                    state.info.mode,
                    operation.cloned(),
                ),
            )
        };
        for child in children {
            child.begin_host_shutdown_owned(force);
        }
        self.changed.notify_waiters();
        diagnostic.emit();
    }

    pub(crate) fn shutdown_status_owned(&self) -> AgentShutdownStatus {
        let (children, settled, fault) = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            let fault = state.info.metadata_fault.clone().or_else(|| {
                state
                    .creations
                    .values()
                    .find_map(|receipt| creation_save_fault(&receipt.result.steps.session))
            });
            (
                state.sessions.values().cloned().collect::<Vec<_>>(),
                state.creating == 0
                    && !state.retirement_joining
                    && state.creation_committed
                    && state.record_work.is_none()
                    && state.close_running.is_empty()
                    && !state.deleting
                    && state.deleting_sessions.is_empty(),
                fault,
            )
        };
        let mut result = AgentShutdownStatus {
            drained: settled,
            quiescent: settled,
            fault,
        };
        for child in children {
            let status: crate::session::SessionShutdownStatus = child.shutdown_status_owned();
            result.drained &= status.drained;
            result.quiescent &= status.quiescent;
            if result.fault.is_none() {
                result.fault = status.fault;
            }
        }
        result.drained &= result.fault.is_none();
        result.quiescent &= result.fault.is_none();
        result
    }

    /// Host owns the shutdown decision; Agent retires only its direct children
    /// and its own storage handles. Persistent business lifecycle is preserved.
    pub(crate) async fn release_host_resources_owned(
        self: &Arc<Self>,
        operation: &OperationId,
    ) -> Result<(), Fault> {
        let status = self.shutdown_status_owned();
        if !status.quiescent {
            return Err(status.fault.unwrap_or_else(|| {
                Fault::new(
                    FaultKind::SessionBusy,
                    "Agent still owns required shutdown work",
                )
            }));
        }
        let (children, mode) = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            if state.resources_released {
                return Ok(());
            }
            (
                state.sessions.values().cloned().collect::<Vec<_>>(),
                state.info.mode,
            )
        };
        for child in children {
            child.release_host_resources_owned().await?;
            if child.storage_mode() == StorageMode::Memory {
                child.reclaim_shutdown_memory_owned()?;
                self.remove_retired_session_owned(&child, Some(operation));
            }
        }
        // No new business may enter after the Host fence. Reconciliation only
        // accepts a retained original range, and there are none at this boundary.
        let tasks = {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            let mut tasks = std::mem::take(&mut state.create_tasks)
                .into_values()
                .collect::<Vec<_>>();
            tasks.extend(std::mem::take(&mut state.close_tasks).into_values());
            tasks.extend(state.record_task.take());
            tasks.extend(state.stop_task.take());
            tasks
        };
        for task in tasks {
            task.await
                .map_err(|_| {
                    Fault::new(
                        FaultKind::AgentInternalUnexpected,
                        "Agent shutdown task did not join cleanly",
                    )
                })
                .inspect_err(|fault| {
                    self.resource_release_failure_record(
                        "shutdown_join",
                        mode,
                        Some(operation),
                        fault,
                    )
                    .emit()
                })?;
        }
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
        if state.info.mode == StorageMode::Memory {
            state.creations.clear();
            state.public_creations.clear();
            state.controls.clear();
            state.info.default_session_id = None;
            state.info.lifecycle = AgentLifecycle::Retired;
        } else if !matches!(
            state.info.lifecycle,
            AgentLifecycle::Retiring | AgentLifecycle::Retired
        ) {
            state.info.lifecycle = AgentLifecycle::Stopped;
        }
        self.changed.notify_waiters();
        let diagnostic = self.lifecycle_record(
            if state.info.mode == StorageMode::Memory {
                "agent.retired"
            } else {
                "agent.resources_released"
            },
            match state.info.lifecycle {
                AgentLifecycle::Creating => "creating",
                AgentLifecycle::Active => "active",
                AgentLifecycle::Stopping => "stopping",
                AgentLifecycle::Stopped => "stopped",
                AgentLifecycle::Retiring => "retiring",
                AgentLifecycle::Retired => "retired",
            },
            state.info.mode,
            Some(operation.clone()),
        );
        drop(state);
        diagnostic.emit();
        Ok(())
    }
}
