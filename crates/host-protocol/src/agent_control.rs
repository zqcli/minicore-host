//! Agent start/stop and default-Session control requests with their receipts.
use crate::{AgentId, OperationId, SessionId, control_receipt};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStartRequest {
    pub operation_id: OperationId,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStartLifecycle {
    Active,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStartResult {
    pub lifecycle: AgentStartLifecycle,
}
control_receipt!(
    AgentStartReceipt,
    AgentStartKind,
    AgentStart,
    "agent.start",
    AgentStartResult,
    agent_id: AgentId
);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStopRequest {
    pub operation_id: OperationId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStopLifecycle {
    Stopping,
    Stopped,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStopResult {
    pub lifecycle: AgentStopLifecycle,
}
crate::control_receipt!(AgentStopReceipt, AgentStopKind, AgentStop, "agent.stop", AgentStopResult, agent_id: AgentId);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSetDefaultSessionRequest {
    pub operation_id: OperationId,
    /// Explicit null clears the default. Omission cannot select another target.
    #[serde(deserialize_with = "crate::nullable")]
    pub session_id: Option<SessionId>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentSetDefaultSessionResult {
    Saving {
        #[serde(deserialize_with = "crate::nullable")]
        default_session_id: Option<SessionId>,
        #[serde(deserialize_with = "crate::nullable")]
        requested_session_id: Option<SessionId>,
    },
    Committed {
        #[serde(deserialize_with = "crate::nullable")]
        default_session_id: Option<SessionId>,
    },
}

control_receipt!(
    AgentSetDefaultSessionReceipt,
    AgentSetDefaultSessionKind,
    AgentSetDefaultSession,
    "agent.set_default_session",
    AgentSetDefaultSessionResult,
    agent_id: AgentId
);
