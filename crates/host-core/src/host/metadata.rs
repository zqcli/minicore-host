use super::*;
impl Host {
    pub fn agent_update(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        request: AgentUpdateRequest,
    ) -> Result<AgentUpdateReceipt, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .update_control(caller, request)
    }
    pub fn session_update(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionUpdateRequest,
    ) -> Result<SessionUpdateReceipt, Fault> {
        self.session(caller, target)?
            .update_control(caller, request)
    }
    pub fn agent_set_default_session(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        request: AgentSetDefaultSessionRequest,
    ) -> Result<AgentSetDefaultSessionReceipt, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .set_default_control(caller, request)
    }

    pub fn load_session(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.session_load(
            caller,
            target,
            SessionControlRequest {
                operation_id: Self::operation_id()?,
            },
        )
        .map(|_| ())
    }
    pub async fn update_session_async(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        expected_revision: u64,
        patch: &SessionConfigPatch,
    ) -> Result<SessionInfo, Fault> {
        self.session(caller, target)?
            .update_async(caller, expected_revision, patch)
            .await
    }
    pub async fn update_agent_async(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        expected_revision: u64,
        patch: &AgentConfigPatch,
    ) -> Result<AgentInfo, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .update_async(caller, expected_revision, patch)
            .await
    }
    pub async fn set_default_session_async(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        session_id: Option<SessionId>,
    ) -> Result<(), Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .set_default_async(caller, session_id)
            .await
    }
    pub async fn reconcile_session_record(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionInfo, Fault> {
        self.session(caller, target)?.reconcile_record(caller).await
    }
    pub async fn reconcile_agent_record(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<AgentInfo, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .reconcile_record(caller)
            .await
    }
}
