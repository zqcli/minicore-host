use super::*;

impl Host {
    pub fn session_delete(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionDeleteRequest,
    ) -> Result<SessionDeleteReceipt, Fault> {
        let receipt = self.agent(caller, &target.agent_id)?.delete_session(
            caller,
            &target.session_id,
            request,
        )?;
        self.ensure_retirement_watch();
        Ok(receipt)
    }
    pub fn agent_delete(
        self: &Arc<Self>,
        caller: &CallerContext,
        target: &AgentRef,
        request: AgentDeleteRequest,
    ) -> Result<AgentDeleteReceipt, Fault> {
        self.check_host(&target.host_id)?;
        request
            .validate()
            .map_err(|error| Fault::new(FaultKind::AgentDeleteInvalid, error.to_string()))?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "deletion requires a live Tokio runtime",
            )
        })?;
        self.access.accept_execution(caller, || {
            let agent = {
                let state = self.state.lock().expect("Host mutex poisoned");
                if !matches!(
                    state.lifecycle,
                    HostLifecycle::Running | HostLifecycle::Draining
                ) {
                    return Err(Fault::new(
                        FaultKind::HostLifecycleUnavailable,
                        "Host cannot accept deletion during retirement",
                    ));
                }
                if let Some(fault) = state.load_failures.get(&target.agent_id) {
                    return Err(fault.clone());
                }
                state.agents.get(&target.agent_id).cloned().ok_or_else(|| {
                    Fault::new(
                        FaultKind::HostAgentNotFound,
                        "Agent does not belong to this Host",
                    )
                })?
            };
            // No Host state lock is held while the direct Agent fixes the scope.
            let children = agent.claim_agent_delete_owned(&request.scope)?;
            let receipt = AgentDeleteReceipt {
                kind: AgentDeleteKind::AgentDelete,
                host_id: self.id.clone(),
                host_incarnation: self.incarnation.clone(),
                agent_id: target.agent_id.clone(),
                operation_id: request.operation_id,
                result: AgentDeleteAccepted {
                    lifecycle: AgentDeleteLifecycle::Retiring,
                    scope: request.scope.clone(),
                },
            };
            let owner = Arc::clone(self);
            let task_id = target.agent_id.clone();
            let mut state = self.state.lock().expect("Host mutex poisoned");
            state.deleting += 1;
            state
                .controls
                .entry(receipt.operation_id.clone())
                .or_default()
                .push(Receipt::AgentDelete(receipt.clone()));
            let task = runtime.spawn(async move {
                agent.delete_agent_owned(request.scope, children).await;
                owner.remove_retired_agents_owned();
                {
                    let mut state = owner.state.lock().expect("Host mutex poisoned");
                    state.deleting -= 1;
                    state.delete_tasks.remove(&task_id);
                }
                owner.changed.notify_waiters();
                owner.publish_host_state_owned();
            });
            state.delete_tasks.insert(target.agent_id.clone(), task);
            Ok(receipt)
        })
    }
}
