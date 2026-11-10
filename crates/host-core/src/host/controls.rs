use super::*;

impl Host {
    pub fn input_remove(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: InputRemoveRequest,
    ) -> Result<InputRemoveReceipt, Fault> {
        self.session(caller, target)?
            .remove_control(caller, request)
    }

    pub(super) fn operation_id() -> Result<OperationId, Fault> {
        OperationId::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))
    }
    pub fn agent_start(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        request: AgentStartRequest,
    ) -> Result<AgentStartReceipt, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .set_enabled(caller, true, Some(request.operation_id))
            .map(|receipt| match receipt {
                Some(Receipt::AgentStart(receipt)) => receipt,
                _ => unreachable!("requested start receipt"),
            })
    }
    pub fn agent_stop(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        request: AgentStopRequest,
    ) -> Result<AgentStopReceipt, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .set_enabled(caller, false, Some(request.operation_id))
            .map(|receipt| match receipt {
                Some(Receipt::AgentStop(receipt)) => receipt,
                _ => unreachable!("requested stop receipt"),
            })
    }
    pub fn session_pause(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionControlRequest,
    ) -> Result<SessionControlReceipt, Fault> {
        self.session(caller, target)?
            .control(caller, request, SessionControlKind::Pause)
    }
    pub fn session_resume(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionControlRequest,
    ) -> Result<SessionControlReceipt, Fault> {
        self.session(caller, target)?
            .control(caller, request, SessionControlKind::Resume)
    }
    pub fn session_load(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionControlRequest,
    ) -> Result<SessionControlReceipt, Fault> {
        self.session(caller, target)?
            .control(caller, request, SessionControlKind::Load)
    }
    pub fn session_unload(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionControlRequest,
    ) -> Result<SessionControlReceipt, Fault> {
        self.session(caller, target)?
            .control(caller, request, SessionControlKind::Unload)
    }
    pub fn session_compact(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: CompactRequest,
    ) -> Result<CompactReceipt, Fault> {
        self.session(caller, target)?.compact(caller, request)
    }
    pub fn session_compact_cancel(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: CompactCancelRequest,
    ) -> Result<CompactCancelReceipt, Fault> {
        self.session(caller, target)?
            .compact_cancel(caller, request)
    }
    pub fn turn_cancel(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: TurnCancelRequest,
    ) -> Result<TurnCancelReceipt, Fault> {
        let (_, receipt) = self.session(caller, target)?.cancel(
            caller,
            Some(&request.turn_id),
            request.keep_inbox,
            Some(request.operation_id),
        )?;
        match receipt {
            Some(Receipt::TurnCancel(receipt)) => Ok(receipt),
            _ => unreachable!("requested Turn receipt"),
        }
    }
    pub fn session_cancel(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionCancelRequest,
    ) -> Result<SessionCancelReceipt, Fault> {
        let (_, receipt) = self.session(caller, target)?.cancel(
            caller,
            None,
            request.keep_inbox,
            Some(request.operation_id),
        )?;
        match receipt {
            Some(Receipt::SessionCancel(receipt)) => Ok(receipt),
            _ => unreachable!("requested Session receipt"),
        }
    }
    pub fn session_close(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SessionCloseRequest,
    ) -> Result<SessionCloseReceipt, Fault> {
        let agent = self.agent(caller, &target.agent_id)?;
        let receipt = agent.close_session(caller, &target.session_id, request)?;
        self.ensure_retirement_watch();
        agent.try_retire_empty_owned(Some(&receipt.operation_id));
        self.remove_retired_agents_owned();
        Ok(receipt)
    }
}

impl Host {
    pub fn answer_interaction(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: InteractionAnswerRequest<crate::InteractionAnswerData>,
    ) -> Result<InteractionAnswerReceipt, Fault> {
        self.session(caller, target)?
            .answer_interaction(caller, request)
    }
}
