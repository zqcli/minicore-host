use super::*;

impl AgentOwner {
    pub(super) fn stop_receipt(
        &self,
        state: &mut AgentState,
        operation: Option<OperationId>,
    ) -> Option<AgentStopReceipt> {
        operation.map(|operation_id| {
            let receipt = AgentStopReceipt {
                kind: AgentStopKind::AgentStop,
                host_id: self.host_id.clone(),
                host_incarnation: self.incarnation.clone(),
                operation_id,
                agent_id: self.id.clone(),
                result: AgentStopResult {
                    lifecycle: match state.info.lifecycle {
                        AgentLifecycle::Stopped => AgentStopLifecycle::Stopped,
                        AgentLifecycle::Stopping => AgentStopLifecycle::Stopping,
                        _ => unreachable!("stop acceptance registered its lifecycle"),
                    },
                },
            };
            state
                .controls
                .entry(receipt.operation_id.clone())
                .or_default()
                .push(receipt.clone());
            receipt
        })
    }
    pub(crate) fn receipts_owned(&self, operation: &OperationId) -> Vec<Receipt> {
        let (mut receipts, children) = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            if state.info.lifecycle == AgentLifecycle::Retired {
                return Vec::new();
            }
            let mut receipts = state
                .creations
                .iter()
                .filter(|(id, receipt)| {
                    state.public_creations.contains(*id) && &receipt.operation_id == operation
                })
                .map(|(_, receipt)| Receipt::SessionCreate(receipt.clone()))
                .collect::<Vec<_>>();
            receipts.extend(
                state
                    .controls
                    .get(operation)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .map(Receipt::AgentStop),
            );
            (
                receipts,
                state.sessions.values().cloned().collect::<Vec<_>>(),
            )
        };
        for child in children {
            receipts.extend(child.control_receipts_owned(operation));
        }
        receipts
    }
    pub(crate) fn retain_session_creation(&self, key: SessionId, receipt: SessionCreateReceipt) {
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        state.public_creations.insert(key.clone());
        state.creations.insert(key, receipt);
    }
}
