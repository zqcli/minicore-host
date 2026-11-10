//! Ordinary reads resolve through each direct owner without loading execution.
use super::*;
use crate::{
    MessageSnapshot, SessionContextSnapshot, TurnDetailsSnapshot, TurnListSnapshot,
    TurnResultSnapshot,
};

impl Host {
    pub fn info(&self, caller: &CallerContext) -> Result<HostInfo, Fault> {
        let control = self.access.snapshot(caller)?;
        let (name, capabilities, resources, facilities) = self.discovery_info();
        let state = self.state.lock().expect("Host mutex poisoned");
        Ok(HostInfo {
            name,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            capabilities,
            resources,
            facilities,
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            lifecycle: state.lifecycle,
            shutdown_strategy: state.shutdown_strategy,
            shutdown_fault: state.shutdown_fault.clone(),
            control,
            logging: self.logging_status(),
        })
    }
    pub fn receipt(
        &self,
        caller: &CallerContext,
        operation_id: &OperationId,
    ) -> Result<AgentCreateReceipt, Fault> {
        let mut receipts = self.creation_receipts(caller, operation_id)?.into_iter();
        let receipt = receipts.next().ok_or_else(|| {
            Fault::new(
                FaultKind::HostReceiptNotFound,
                "no retained creation receipt for this operation",
            )
        })?;
        if receipts.next().is_some() {
            return Err(Fault::new(
                FaultKind::HostReceiptAmbiguous,
                "operation_id matches multiple invocations; use creation_receipts",
            ));
        }
        Ok(receipt)
    }
    /// All accepted invocations with this correlation ID. Reusing an operation
    /// ID neither deduplicates a request nor replaces another invocation's facts.
    pub fn creation_receipts(
        &self,
        caller: &CallerContext,
        operation_id: &OperationId,
    ) -> Result<Vec<AgentCreateReceipt>, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Host mutex poisoned")
                .creations
                .values()
                .filter(|receipt| &receipt.operation_id == operation_id)
                .cloned()
                .collect())
        })
    }
    pub fn agent_info(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<AgentInfo, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?.info(caller)
    }
    pub fn session_info(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionInfo, Fault> {
        self.session(caller, target)?.info(caller)
    }
    pub fn session_state(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionSnapshot, Fault> {
        self.session(caller, target)?.snapshot(caller)
    }
    pub fn message(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        message_id: &MessageId,
    ) -> Result<MessageRecord, Fault> {
        self.session(caller, target)?.message(caller, message_id)
    }
    pub fn history(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<Vec<HistoryItem>, Fault> {
        self.session(caller, target)?.history(caller)
    }
    pub fn turns(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<Vec<TurnInfo>, Fault> {
        self.session(caller, target)?.turns(caller)
    }
    pub fn turn_info(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        id: &TurnId,
    ) -> Result<TurnInfo, Fault> {
        self.session(caller, target)?.turn_info(caller, id)
    }
    pub fn turn_result(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        self.session(caller, target)?.turn_result(caller, id)
    }
    pub async fn wait_turn(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        // The observation token contains only a Weak owner and a bounded state
        // snapshot. An abandoned wait cannot retain Session bodies or runners.
        let mut states = self.session_states(caller, target)?;
        loop {
            let turn = self.turn_info(caller, target, id)?;
            if turn.state == TurnState::Finished {
                return self.turn_result(caller, target, id);
            }
            states.changed().await?;
        }
    }
    pub fn turn_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
    ) -> Result<TurnDetailsSnapshot, Fault> {
        self.session(caller, target)?.turn_details(caller, turn_id)
    }
    pub fn message_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        message_id: &MessageId,
    ) -> Result<MessageSnapshot, Fault> {
        self.session(caller, target)?
            .message_snapshot(caller, message_id)
    }
    pub fn turn_list(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<TurnListSnapshot, Fault> {
        self.session(caller, target)?.turn_list(caller)
    }
    pub async fn turn_result_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
    ) -> Result<TurnResultSnapshot, Fault> {
        self.session(caller, target)?
            .turn_result_snapshot(caller, turn_id)
            .await
    }
    pub async fn wait_turn_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
    ) -> Result<TurnResultSnapshot, Fault> {
        // A saved terminal result needs no execution subscription or waiter.
        if self
            .session(caller, target)?
            .turn_query_finished(caller, turn_id)?
            .unwrap_or(true)
        {
            return self.turn_result_snapshot(caller, target, turn_id).await;
        }
        // Subscribe before checking again, so completion cannot be missed.
        // The existing subscription holds only a Weak owner and light state.
        // Dropping this wait cannot retain content or cancel accepted work.
        let mut states = self.session_states(caller, target)?;
        loop {
            if self
                .session(caller, target)?
                .turn_query_finished(caller, turn_id)?
                .unwrap_or(true)
            {
                return self.turn_result_snapshot(caller, target, turn_id).await;
            }
            states.changed().await?;
        }
    }
    pub async fn history_read(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: &HistoryReadRequest,
    ) -> Result<SessionHistorySnapshot, Fault> {
        self.session(caller, target)?
            .history_read(caller, request)
            .await
    }
    pub fn turn_output_read(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
        response_index: u64,
        request: &TextReadRequest,
    ) -> Result<ModelTextSnapshot, Fault> {
        let session = self.session(caller, target)?;
        let info = session.turn_info(caller, turn_id)?;
        let data = session.output_read(caller, turn_id, Some(response_index), None, request)?;
        Ok(ModelTextSnapshot {
            source: ModelOutputIdentity {
                turn: TurnIdentity {
                    session: SessionIdentity {
                        host_id: self.id.clone(),
                        host_incarnation: self.incarnation.clone(),
                        agent_id: target.agent_id.clone(),
                        session_id: target.session_id.clone(),
                    },
                    turn_id: turn_id.clone(),
                    load_id: Some(info.load_id),
                },
                response_index,
            },
            data,
        })
    }
    pub fn toolcall_output_read(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        tool: &ToolCallRef,
        request: &TextReadRequest,
    ) -> Result<ToolTextSnapshot, Fault> {
        let session = self.session(caller, target)?;
        let (source, _) = session.tool_get(caller, &tool.turn_id, &tool.tool_call_id)?;
        let data = session.output_read(
            caller,
            &tool.turn_id,
            None,
            Some(&tool.tool_call_id),
            request,
        )?;
        Ok(ToolTextSnapshot { source, data })
    }
    pub fn toolcall_get(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        tool: &ToolCallRef,
    ) -> Result<ToolCallSnapshot, Fault> {
        let (source, data) =
            self.session(caller, target)?
                .tool_get(caller, &tool.turn_id, &tool.tool_call_id)?;
        Ok(ToolCallSnapshot { source, data })
    }
    /// Read context usage, compaction and execution facts through the actual
    /// Agent and Session owners. Cold, never-loaded history remains unknown.
    pub async fn session_context(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionContextSnapshot, Fault> {
        self.session(caller, target)?.context(caller).await
    }
    pub fn input_list(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<InputList, Fault> {
        self.session(caller, target)?.input_list(caller)
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
                receipts.extend(state.controls.get(operation).into_iter().flatten().cloned());
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
