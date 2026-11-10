use super::*;

pub(crate) struct SessionShutdownStatus {
    pub(crate) drained: bool,
    pub(crate) quiescent: bool,
    pub(crate) fault: Option<Fault>,
}
impl SessionController {
    pub(super) async fn prepare_resources(
        &self,
        config: &SessionConfig,
        binding: crate::PreparedTurnBinding,
    ) -> Result<BoundResources, Fault> {
        let resources = self.resources_owned()?;
        let preparation = resources.resolve_for_turn(config, binding);
        tokio::pin!(preparation);
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.state.lock().expect("Session mutex poisoned");
                if state.deleting {
                    return Err(Fault::new(
                        FaultKind::SessionLifecycleClosing,
                        "Session deletion fenced unfinished Turn preparation",
                    ));
                }
                if state.host_stopping || !self.access.execution_allowed_owned() {
                    return Err(Fault::new(
                        FaultKind::HostLifecycleUnavailable,
                        "Host stopped before resource preparation completed",
                    ));
                }
            }
            tokio::select! {
                result = &mut preparation => return result,
                _ = changed => {}
            }
        }
    }

    pub(super) fn resources_owned(&self) -> Result<Arc<dyn ResourceResolver>, Fault> {
        self.resources
            .lock()
            .expect("Session resources mutex poisoned")
            .clone()
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Session resources have been retired",
                )
            })
    }
    pub(super) fn store_owned(&self) -> Option<SessionStore> {
        self.store
            .lock()
            .expect("Session store mutex poisoned")
            .clone()
    }
    pub(super) fn ensure_visible(&self) -> Result<(), Fault> {
        let state = self.state.lock().expect("Session mutex poisoned");
        if state.reclaimed {
            Err(Fault::new(
                FaultKind::AgentSessionNotFound,
                "memory Session has been reclaimed",
            ))
        } else {
            Ok(())
        }
    }
    pub(crate) fn begin_host_shutdown_owned(&self, force: bool) {
        let (handle, record) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.host_draining && (!force || state.host_stopping) {
                return;
            }
            let claimed = force && Self::cancellable(&state);
            state.host_draining = true;
            state.host_stopping |= force;
            if force {
                Self::cancel_compaction_locked(&mut state);
                self.start_scope_cleanup_locked(&mut state, false);
            }
            let handle = if state.host_stopping {
                clear_inbox(&mut state);
                state.active.as_mut().and_then(|active| {
                    if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                        active._activation.expire();
                        active.info.state = TurnState::Cancelling;
                    }
                    active.handle.clone()
                })
            } else {
                None
            };
            self.publish(&mut state);
            let record = self.cancel_claim_record(&state, claimed, "host_force", None, true);
            (handle, record)
        };
        diagnostic::emit(record);
        // Runtime shutdown cancels business work and still joins through the
        // original owner. It never aborts the history callback or driver future.
        if let Some(handle) = handle {
            handle.shutdown();
        }
        self.handoff_delegation_owned();
    }
    pub(crate) fn shutdown_status_owned(&self) -> SessionShutdownStatus {
        let state = self.state.lock().expect("Session mutex poisoned");
        let quiescent = Self::work_settled(&state);
        let drained = quiescent && state.next_step.is_empty() && state.next_turn.is_empty();
        let fault = state
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
            .or_else(|| (!drained).then(|| state.block_reason.clone()).flatten());
        SessionShutdownStatus {
            drained,
            quiescent,
            fault,
        }
    }
    pub(super) fn work_settled(state: &SessionState) -> bool {
        state.active.is_none()
            && state.compaction.is_none()
            && !state.preparing
            && !state.driver_running
            && state.history_work.is_none()
            && state.record_work.is_none()
            && state.close_fault.is_none()
            && !state.handoff_pending
            && !state.scope_cleanup_pending
            && !state.delegation.as_ref().is_some_and(|slot| slot.pending())
            && state
                .root_scope
                .as_ref()
                .is_none_or(|scope| scope.pending.load(Ordering::SeqCst) == 0)
    }
    /// This is resource retirement, not business close and not an acknowledgement
    /// shortcut. The direct Agent calls it only after its accepted shutdown fence.
    pub(crate) async fn release_host_resources_owned(self: &Arc<Self>) -> Result<(), Fault> {
        self.release_resources(false).await?;
        // A failed explicit delete can retain only its actual storage locator
        // after execution/history resources were already released. Controlled
        // Host retirement must release that locator too, without deleting data.
        drop(
            self.store
                .lock()
                .expect("Session store mutex poisoned")
                .take(),
        );
        self.scope_root_closed_owned();
        Ok(())
    }
    /// Only the built-in no-resource MemoryHistory may retire without an async
    /// worker. Custom backends still receive their explicit release callback.
    pub(crate) fn retire_closed_immediately_owned(self: &Arc<Self>) -> Result<bool, Fault> {
        if self.backend.as_ref().type_id() != std::any::TypeId::of::<MemoryHistory>() {
            return Ok(false);
        }
        let tasks = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.reclaimed {
                return Ok(true);
            }
            if state.info.mode != StorageMode::Memory
                || state.lifecycle != SessionLifecycle::Closed
                || state.releasing
                || !Self::work_settled(&state)
                || [
                    &state.driver_task,
                    &state.load_task,
                    &state.record_task,
                    &state.history_task,
                    &state.compaction_task,
                    &state.scope_task,
                ]
                .into_iter()
                .any(|task| task.as_ref().is_some_and(|task| !task.is_finished()))
            {
                return Ok(false);
            }
            state.releasing = true;
            [
                state.driver_task.take(),
                state.load_task.take(),
                state.record_task.take(),
                state.history_task.take(),
                state.compaction_task.take(),
                state.scope_task.take(),
            ]
        };
        for task in tasks.into_iter().flatten() {
            if !matches!(task.now_or_never(), Some(Ok(()))) {
                let fault = Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "memory Session worker did not retire normally",
                );
                let mut state = self.state.lock().expect("Session mutex poisoned");
                state.releasing = false;
                state.close_fault = Some(fault.clone());
                self.publish(&mut state);
                return Err(fault);
            }
        }
        let store = self
            .store
            .lock()
            .expect("Session store mutex poisoned")
            .take();
        let lease = self
            .root_lease
            .lock()
            .expect("Session lease mutex poisoned")
            .take();
        let resources = self
            .resources
            .lock()
            .expect("Session resources mutex poisoned")
            .take();
        drop(store);
        drop(lease);
        drop(resources);
        {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            state.releasing = false;
            state.released = true;
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
        self.scope_root_closed_owned();
        self.reclaim_bodies();
        Ok(true)
    }
    /// Host stopped ends a memory entity's running lifetime, independently of
    /// business close. The direct Agent invokes this only after actual release.
    pub(crate) fn reclaim_shutdown_memory_owned(&self) -> Result<(), Fault> {
        {
            let state = self.state.lock().expect("Session mutex poisoned");
            if state.info.mode != StorageMode::Memory || !state.released || state.releasing {
                return Err(Fault::new(
                    FaultKind::SessionBusy,
                    "shutdown memory reclamation requires completed resource retirement",
                ));
            }
        }
        self.reclaim_bodies();
        Ok(())
    }
    pub(crate) async fn retire_closed_owned(self: &Arc<Self>) -> Result<(), Fault> {
        self.release_resources(true).await?;
        self.scope_root_closed_owned();
        self.reclaim_bodies();
        Ok(())
    }
    pub(super) fn reclaim_bodies(&self) {
        let retired = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            state.reclaimed = true;
            state.controls.clear();
            state.finished_manual_compactions.clear();
            let retired = (
                std::mem::take(&mut state.output),
                std::mem::replace(
                    &mut state.edits,
                    minicore_host_workspace::EditJournal::new(self.id.clone()),
                ),
                std::mem::take(&mut state.history),
                std::mem::take(&mut state.input_provenance),
                state.readable_history.take(),
                std::mem::take(&mut state.messages),
                std::mem::take(&mut state.completed),
                std::mem::take(&mut state.completion_order),
            );
            state.projection = None;
            state.last_compaction = None;
            state.history_loaded = false;
            self.publish(&mut state);
            retired
        };
        drop(retired);
    }
    pub(super) async fn release_resources(
        self: &Arc<Self>,
        closed_memory: bool,
    ) -> Result<(), Fault> {
        let tasks = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if closed_memory {
                if (state.info.mode != StorageMode::Memory && !state.deleting)
                    || state.lifecycle != SessionLifecycle::Closed
                {
                    return Err(Fault::new(
                        FaultKind::SessionBusy,
                        "release requires closed memory or explicitly deleted persistent ownership",
                    ));
                }
            } else if !state.host_draining
                || (!state.host_stopping
                    && (!state.next_step.is_empty() || !state.next_turn.is_empty()))
            {
                return Err(Fault::new(
                    FaultKind::SessionBusy,
                    "Session retirement has no accepted shutdown fence",
                ));
            }
            if state.released {
                return Ok(());
            }
            if state.releasing || !Self::work_settled(&state) {
                return Err(state
                    .history_work
                    .as_ref()
                    .and_then(|work| work.fault.clone())
                    .or_else(|| {
                        state
                            .record_work
                            .as_ref()
                            .and_then(|work| work.fault.clone())
                    })
                    .or_else(|| {
                        state
                            .active
                            .as_ref()
                            .and_then(|active| active.tool_cleanup.fault.clone())
                    })
                    .unwrap_or_else(|| {
                        Fault::new(
                            FaultKind::SessionBusy,
                            "Session still owns execution, saving, or retirement work",
                        )
                    }));
            }
            state.releasing = true;
            [
                state.driver_task.take(),
                state.load_task.take(),
                state.record_task.take(),
                state.history_task.take(),
                state.compaction_task.take(),
                state.scope_task.take(),
            ]
        };
        // State acknowledgements can precede task return. Join every retained
        // task before dropping storage/resources and releasing directory ownership.
        for task in tasks.into_iter().flatten() {
            if task.await.is_err() {
                let fault = Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "Session worker did not retire normally",
                );
                let mut state = self.state.lock().expect("Session mutex poisoned");
                state.releasing = false;
                state.close_fault = Some(fault.clone());
                self.publish(&mut state);
                return Err(fault);
            }
        }
        let released = AssertUnwindSafe(async { self.backend.release().await })
            .catch_unwind()
            .await
            .unwrap_or_else(|_| {
                Err(Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "history resource retirement panicked",
                ))
            });
        if let Err(fault) = released {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            state.releasing = false;
            state.close_fault = Some(fault.clone());
            self.publish(&mut state);
            return Err(fault);
        }
        let store = self
            .store
            .lock()
            .expect("Session store mutex poisoned")
            .take();
        let lease = self
            .root_lease
            .lock()
            .expect("Session lease mutex poisoned")
            .take();
        let resources = self
            .resources
            .lock()
            .expect("Session resources mutex poisoned")
            .take();
        drop(store);
        drop(lease);
        drop(resources);
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.releasing = false;
        state.released = true;
        state.residency = Residency::Unloaded;
        state.load_id = None;
        self.publish(&mut state);
        Ok(())
    }
}
