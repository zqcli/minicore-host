use super::*;

impl Host {
    pub(super) fn operation_id() -> Result<OperationId, Fault> {
        OperationId::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))
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
            .map(|receipt| receipt.expect("requested stop receipt"))
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
        agent.try_retire_empty_owned();
        self.remove_retired_agents_owned();
        Ok(receipt)
    }
    /// Lookup traverses the current direct-owner indexes, without retaining
    /// owners or copying Session facts into a Host result cache.
    pub fn receipts(
        &self,
        caller: &CallerContext,
        operation: &OperationId,
    ) -> Result<Vec<Receipt>, Fault> {
        self.access.read(caller, || {
            let (mut receipts, children) = {
                let state = self.state.lock().expect("Host mutex poisoned");
                let mut receipts = state
                    .creations
                    .values()
                    .filter(|receipt| &receipt.operation_id == operation)
                    .cloned()
                    .map(Receipt::AgentCreate)
                    .collect::<Vec<_>>();
                receipts.extend(
                    state
                        .controls
                        .get(operation)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .map(Receipt::HostShutdown),
                );
                (receipts, state.agents.values().cloned().collect::<Vec<_>>())
            };
            receipts.extend(self.configuration.receipts_owned(operation));
            for child in children {
                receipts.extend(child.receipts_owned(operation));
            }
            Ok(receipts)
        })
    }
    /// An operation ID is correlation only. Ambiguous independent invocations
    /// are never merged, deduplicated, overwritten, or replayed.
    pub fn receipt_get(
        &self,
        caller: &CallerContext,
        operation: &OperationId,
    ) -> Result<Receipt, Fault> {
        let mut receipts = self.receipts(caller, operation)?.into_iter();
        let receipt = receipts.next().ok_or_else(|| {
            Fault::new(
                FaultKind::HostReceiptNotFound,
                "no owner retains facts for this operation",
            )
        })?;
        if receipts.next().is_some() {
            return Err(Fault::new(
                FaultKind::HostReceiptAmbiguous,
                "operation_id matches multiple invocations; use receipts to inspect their separate facts",
            ));
        }
        Ok(receipt)
    }
}
