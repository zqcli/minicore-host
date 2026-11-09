use super::*;

impl Host {
    pub(super) fn ensure_retirement_watch(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let mut state = self.state.lock().expect("Host mutex poisoned");
        if state
            .retirement_watch
            .as_ref()
            .is_some_and(|task| !task.is_finished())
            || matches!(
                state.lifecycle,
                HostLifecycle::Stopping | HostLifecycle::Stopped
            )
        {
            return;
        }
        let owner = self.self_weak.clone();
        let changed = Arc::clone(&self.changed);
        state.retirement_watch = Some(runtime.spawn(async move {
            loop {
                let notified = changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let Some(host) = owner.upgrade() else {
                    return;
                };
                let agents = {
                    let state = host.state.lock().expect("Host mutex poisoned");
                    if matches!(
                        state.lifecycle,
                        HostLifecycle::Stopping | HostLifecycle::Stopped
                    ) {
                        return;
                    } else {
                        state.agents.values().cloned().collect::<Vec<_>>()
                    }
                };
                for agent in &agents {
                    agent.reap_closed_sessions_owned().await;
                }
                host.remove_retired_agents_owned();
                if !agents
                    .iter()
                    .any(|agent| agent.memory_retirement_pending_owned())
                {
                    return;
                }
                drop(agents);
                // Idle observation never keeps Host or directory ownership alive.
                drop(host);
                notified.await;
            }
        }));
    }
    pub(super) fn remove_retired_agents_owned(&self) {
        let agents = self
            .state
            .lock()
            .expect("Host mutex poisoned")
            .agents
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let retired = agents
            .into_iter()
            .filter(|agent| agent.is_retired_owned())
            .collect::<Vec<_>>();
        let removed = {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            retired
                .iter()
                .filter_map(|agent| {
                    if state
                        .agents
                        .get(&agent.id)
                        .is_some_and(|current| Arc::ptr_eq(current, agent))
                    {
                        state.agents.remove(&agent.id)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        drop(removed);
    }
}
