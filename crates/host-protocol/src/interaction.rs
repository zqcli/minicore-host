//! Answer an existing Runtime wait, not a message or a new execution.
use crate::*;
use serde::{Deserialize, Serialize};

/// The Session target is provided by the same typed/RPC owner context as other
/// Session methods. `A` is the execution adapter's registered answer payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionAnswerRequest<A> {
    pub operation_id: OperationId,
    pub turn_id: TurnId,
    pub load_id: LoadId,
    pub interaction_id: InteractionId,
    pub answer: A,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InteractionAnswerKind {
    #[serde(rename = "interaction.answer")]
    InteractionAnswer,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionAnswerStatus {
    Answered,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionAnswerResult {
    pub status: InteractionAnswerStatus,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionAnswerReceipt {
    pub kind: InteractionAnswerKind,
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub load_id: LoadId,
    pub interaction_id: InteractionId,
    pub operation_id: OperationId,
    pub result: InteractionAnswerResult,
}
