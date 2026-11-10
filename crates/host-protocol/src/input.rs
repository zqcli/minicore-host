//! The Session owner's two queues and explicit removal of unconsumed input.
use crate::{
    AgentId, ControllerRevision, HostId, HostIncarnation, InputQueue, MessageId, MessageInput,
    MessageSource, OperationId, SessionId, ValidationError, control_receipt, invalid,
    validated_dto,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedInputRecord {
    pub input: MessageInput,
    pub source: MessageSource,
    pub queue: InputQueue,
    pub order: u64,
}
validated_dto!(InputList, InputListWire, "InputListWire" {
    host_id: HostId,
    host_incarnation: HostIncarnation,
    agent_id: AgentId,
    session_id: SessionId,
    controller_revision: ControllerRevision,
    next_step: Vec<QueuedInputRecord>,
    next_turn: Vec<QueuedInputRecord>,
});
impl InputList {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (queue, entries) in [
            (InputQueue::NextStep, &self.next_step),
            (InputQueue::NextTurn, &self.next_turn),
        ] {
            if entries.iter().any(|entry| entry.queue != queue)
                || entries
                    .windows(2)
                    .any(|pair| pair[0].order >= pair[1].order)
            {
                return Err(invalid(
                    "input entries must retain their queue and increasing queue order",
                ));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRemoveRequest {
    pub operation_id: OperationId,
    pub message_id: MessageId,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub expected_controller_revision: Option<ControllerRevision>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRemoveResult {
    pub queue: InputQueue,
    pub order: u64,
    pub controller_revision: ControllerRevision,
}

control_receipt!(InputRemoveReceipt, InputRemoveKind, InputRemove, "input.remove", InputRemoveResult, agent_id: AgentId, session_id: SessionId, message_id: MessageId);
