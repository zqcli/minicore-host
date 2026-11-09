use super::*;
use futures_util::FutureExt;

impl AgentOwner {
    /// Session closure owns its cleanup. Only this direct Agent may remove the
    /// registered child, after its real work and resources have retired.
    pub(crate) async fn reap_closed_sessions_owned(&self) {
        let children = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            state
                .closing
                .iter()
                .filter(|id| !state.close_running.contains(*id))
                .filter_map(|id| state.sessions.get(id))
                .cloned()
                .collect::<Vec<_>>()
        };
        for child in children {
            if child.storage_mode() != StorageMode::Memory {
                continue;
            }
            if child.retire_closed_owned().await.is_ok() {
                self.remove_retired_session_owned(&child);
            }
        }
        self.join_empty_owner_tasks().await;
        self.try_retire_empty_owned();
    }
    async fn join_empty_owner_tasks(&self) {
        let tasks = {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if state.info.mode != StorageMode::Memory
                || !state.creation_committed
                || !state.sessions.is_empty()
                || state.creating != 0
                || state.record_work.is_some()
                || !state.close_running.is_empty()
                || state.retirement_joining
            {
                return;
            }
            let mut tasks = std::mem::take(&mut state.create_tasks)
                .into_values()
                .collect::<Vec<_>>();
            tasks.extend(std::mem::take(&mut state.close_tasks).into_values());
            tasks.extend(state.stop_task.take());
            tasks.extend(state.record_task.take());
            if tasks.is_empty() {
                return;
            }
            state.retirement_joining = true;
            tasks
        };
        let mut fault = None;
        for task in tasks {
            if task.await.is_err() {
                fault = Some(Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "Agent retirement task did not join cleanly",
                ));
            }
        }
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        state.retirement_joining = false;
        if fault.is_some() {
            state.info.metadata_fault = fault;
        }
        self.changed.notify_waiters();
    }
    pub(super) fn remove_retired_session_owned(&self, child: &Arc<SessionController>) {
        let removed = {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if !state
                .sessions
                .get(&child.id)
                .is_some_and(|current| Arc::ptr_eq(current, child))
            {
                return;
            }
            if state.info.default_session_id.as_ref() == Some(&child.id) {
                state.info.default_session_id = None;
            }
            state.closing.remove(&child.id);
            state.sessions.remove(&child.id)
        };
        drop(removed);
        self.changed.notify_waiters();
    }
    /// Creation admission and this final retiring claim share AgentState. No
    /// request routed before the claim can publish into a retired owner later.
    pub(crate) fn try_retire_empty_owned(&self) {
        let tasks = {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if state.info.mode != StorageMode::Memory
                || !state.creation_committed
                || !state.sessions.is_empty()
                || state.creating != 0
                || state.record_work.is_some()
                || state.info.metadata_fault.is_some()
                || !state.close_running.is_empty()
                || state.retirement_joining
                || state
                    .stop_task
                    .as_ref()
                    .is_some_and(|task| !task.is_finished())
                || state
                    .record_task
                    .as_ref()
                    .is_some_and(|task| !task.is_finished())
                || state.create_tasks.values().any(|task| !task.is_finished())
                || state.close_tasks.values().any(|task| !task.is_finished())
                || matches!(
                    state.info.lifecycle,
                    AgentLifecycle::Creating | AgentLifecycle::Retiring | AgentLifecycle::Retired
                )
            {
                return;
            } else {
                state.info.lifecycle = AgentLifecycle::Retiring;
                self.enabled.store(false, Ordering::SeqCst);
                let mut tasks = std::mem::take(&mut state.create_tasks)
                    .into_values()
                    .collect::<Vec<_>>();
                tasks.extend(std::mem::take(&mut state.close_tasks).into_values());
                tasks.extend(state.stop_task.take());
                tasks.extend(state.record_task.take());
                tasks
            }
        };
        for task in tasks {
            if !matches!(task.now_or_never(), Some(Ok(()))) {
                self.state
                    .lock()
                    .expect("Agent mutex poisoned")
                    .info
                    .metadata_fault = Some(Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "Agent retirement task did not join cleanly",
                ));
                self.changed.notify_waiters();
                return;
            }
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
        {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            state.creations.clear();
            state.public_creations.clear();
            state.controls.clear();
            state.info.default_session_id = None;
            state.resources_released = true;
            state.info.lifecycle = AgentLifecycle::Retired;
        }
        self.changed.notify_waiters();
    }
    pub(crate) fn memory_retirement_pending_owned(&self) -> bool {
        let (children, empty_pending) = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            (
                state
                    .closing
                    .iter()
                    .filter_map(|id| state.sessions.get(id))
                    .cloned()
                    .collect::<Vec<_>>(),
                state.info.mode == StorageMode::Memory
                    && state.creation_committed
                    && state.sessions.is_empty()
                    && (state.creating != 0
                        || state.retirement_joining
                        || state.info.lifecycle == AgentLifecycle::Stopping),
            )
        };
        empty_pending
            || children
                .iter()
                .any(|child| child.storage_mode() == StorageMode::Memory)
    }
    pub(crate) fn is_retired_owned(&self) -> bool {
        self.state
            .lock()
            .expect("Agent mutex poisoned")
            .info
            .lifecycle
            == AgentLifecycle::Retired
    }
}
