use super::*;
use crate::TurnCapability;
use crate::collaboration::*;

impl Host {
    pub(crate) fn prepare_delegation_owned(
        &self,
        turn: &TurnCapability,
        registration: DelegationRegistration,
        sink: Arc<dyn DelegationSink>,
    ) -> Result<DelegationPermit, Fault> {
        let source = turn.source();
        if registration.host_id != source.host_id
            || registration.host_incarnation != source.host_incarnation
            || registration.parent != source.session
            || registration.origin.turn_id != source.turn.turn_id
            || registration.child == registration.root
            || registration.child == registration.parent
        {
            return Err(Fault::new(
                FaultKind::SessionOwnershipMismatch,
                "delegation does not match its bound source and direct relationship",
            ));
        }
        let caller = turn.caller()?;
        let parent = self.session(&caller, &registration.parent)?;
        let child = self.session(&caller, &registration.child)?;
        let scope = parent.scope_owned(&registration.root)?;
        self.access.accept_business(&caller, || {
            scope.check_open()?;
            child.register_delegation_owned(registration, Arc::downgrade(&parent), scope, sink)
        })
    }
    pub(crate) fn send_delegated_owned(
        &self,
        turn: &TurnCapability,
        registration: &DelegationRegistration,
        input: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        let caller = turn.caller()?;
        let source = self.session(&caller, &turn.source().session)?;
        let child = self.session(&caller, &registration.child)?;
        let scope = source.scope_owned(&registration.root)?;
        let (result, record) = self.access.accept_execution(&caller, || {
            scope.check_open()?;
            child.receive_delegated_owned(
                registration,
                &scope,
                turn.source().session.clone(),
                input,
            )
        })?;
        record.emit();
        child.wake_owned();
        Ok(result)
    }
    /// Narrow cleanup entry for the exact target already registered with a
    /// scope. Discovery still follows Host -> Agent -> Session, including close
    /// record/default ownership and the actual child-worker join.
    pub(crate) async fn close_internal_owned(
        &self,
        target: &SessionRef,
        operation: OperationId,
    ) -> Result<(), Fault> {
        let agent = {
            let state = self.state.lock().expect("Host mutex poisoned");
            state.agents.get(&target.agent_id).cloned().ok_or_else(|| {
                Fault::new(
                    FaultKind::HostAgentNotFound,
                    "delegated Agent is no longer available",
                )
            })?
        };
        agent
            .close_internal_owned(&target.session_id, operation)
            .await?;
        self.ensure_retirement_watch();
        self.remove_retired_agents_owned();
        Ok(())
    }
}
impl Host {
    pub(crate) async fn update_agent_candidate_owned(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        expected: u64,
        config: AgentConfig,
        name: String,
    ) -> Result<AgentInfo, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .update_candidate_async(caller, expected, config, name)
            .await
    }
    pub(crate) async fn update_session_candidate_owned(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        expected: u64,
        config: SessionConfig,
        title: Option<String>,
    ) -> Result<SessionInfo, Fault> {
        self.session(caller, target)?
            .update_candidate_async(caller, expected, config, title)
            .await
    }
}
