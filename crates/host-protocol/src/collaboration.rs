//! Recorded delegation facts, not scheduling authority or retained child output.
use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegationAssociation {
    pub parent: SessionRef,
    pub origin: ToolCallRef,
    pub child: SessionRef,
    pub task_message_id: MessageId,
    pub accepted_message_ids: Vec<MessageId>,
    pub inputs: Vec<DelegationInputRange>,
    pub completion: Option<DelegationTerminalData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegationInputRange {
    pub turn: TurnRef,
    /// All actual inputs in this shared Turn, including unrelated inputs.
    pub message_ids: Vec<MessageId>,
    /// The subset accepted through this delegation, including real bindings
    /// cancelled before the first model request.
    pub delegated_message_ids: Vec<MessageId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegationTerminalData {
    pub turns: Vec<DelegationTurnOutcome>,
    pub discarded_inputs: Vec<MessageId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegationTurnOutcome {
    pub turn: TurnRef,
    pub outcome: TurnOutcome,
    pub fault: Option<Fault>,
}
