use crate::{
    AgentId, Fault, HostId, HostIncarnation, HostLifecycle, LoadId, OperationId, Residency,
    Revision, Scheduling, SessionId, TurnId, TurnState, ValidationError, invalid,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;

crate::validated_dto!(#[derive(Default)] CallOptions, CallOptionsWire, "CallOptionsWire" {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    wait_timeout_ms: Option<u64>,
});
impl CallOptions {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.wait_timeout_ms == Some(0) {
            Err(invalid(
                "wait_timeout_ms must be positive integer milliseconds",
            ))
        } else {
            Ok(())
        }
    }
    pub fn timeout_ms(&self) -> u64 {
        self.wait_timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlStateData {
    pub occupied: bool,
    pub revision: Revision,
}
crate::validated_dto!(ConnectionInfo, ConnectionInfoWire, "ConnectionInfoWire" {
    host_id: HostId,
    host_incarnation: HostIncarnation,
    can_operate: bool,
    control: ControlStateData,
});
impl ConnectionInfo {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.can_operate && !self.control.occupied {
            Err(invalid(
                "an operating connection implies occupied Host control",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostShutdownStrategy {
    Graceful,
    Force,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostShutdownRequest {
    pub operation_id: OperationId,
    pub strategy: HostShutdownStrategy,
}
crate::validated_dto!(HostShutdownResult, HostShutdownResultWire, "HostShutdownResultWire" {
    strategy: HostShutdownStrategy,
    lifecycle: HostLifecycle,
});
impl HostShutdownResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match (self.strategy, self.lifecycle) {
            (
                HostShutdownStrategy::Graceful,
                HostLifecycle::Draining | HostLifecycle::Stopping | HostLifecycle::Stopped,
            )
            | (HostShutdownStrategy::Force, HostLifecycle::Stopping | HostLifecycle::Stopped) => {
                Ok(())
            }
            _ => Err(invalid(
                "shutdown requires its actual stopping/draining state; force cannot drain",
            )),
        }
    }
}
crate::control_receipt!(
    HostShutdownReceipt,
    HostShutdownKind,
    HostShutdown,
    "host.shutdown",
    HostShutdownResult
);

/// Requests below use the already-selected Session's trusted typed context.
/// Independent RPC adapters must additionally carry that target SessionRef.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnCancelRequest {
    pub operation_id: OperationId,
    pub turn_id: TurnId,
    #[serde(default)]
    pub keep_inbox: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCancelRequest {
    pub operation_id: OperationId,
    #[serde(default)]
    pub keep_inbox: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCloseRequest {
    pub operation_id: OperationId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionControlRequest {
    pub operation_id: OperationId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStopRequest {
    pub operation_id: OperationId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactRequest {
    pub operation_id: OperationId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactCancelRequest {
    pub operation_id: OperationId,
    pub target_operation_id: OperationId,
}

crate::validated_dto!(TurnCancelResult, TurnCancelResultWire, "TurnCancelResultWire" {
    cancel_claimed: bool,
    turn_state: TurnState,
    controller_revision: Revision,
});
impl TurnCancelResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self.turn_state {
            TurnState::Cancelling | TurnState::Finalizing | TurnState::Finished => Ok(()),
            TurnState::Starting | TurnState::Running => Err(invalid(
                "a cancel receipt must reflect actual cancellation/terminal progress",
            )),
        }
    }
}
crate::control_receipt!(TurnCancelReceipt, TurnCancelKind, TurnCancel, "turn.cancel", TurnCancelResult, agent_id: AgentId, session_id: SessionId, turn_id: TurnId);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCloseLifecycle {
    Closing,
    Closed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCloseResult {
    pub lifecycle: SessionCloseLifecycle,
    pub controller_revision: Revision,
}
crate::control_receipt!(SessionCloseReceipt, SessionCloseKind, SessionClose, "session.close", SessionCloseResult, agent_id: AgentId, session_id: SessionId);

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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    Explicit,
    ExecutionFailure,
    UnloadedWithInput,
}
crate::validated_dto!(BlockReason, BlockReasonWire, "BlockReasonWire" {
    message: String,
    resolution: String,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    fault: Option<Fault>,
});
impl BlockReason {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.message.trim().is_empty() || self.resolution.trim().is_empty() {
            Err(invalid(
                "block_reason requires a meaningful cause and release condition",
            ))
        } else {
            Ok(())
        }
    }
}
crate::validated_dto!(SessionControlFacts, SessionControlFactsWire, "SessionControlFactsWire" {
    scheduling: Scheduling,
    pause_reasons: Vec<PauseReason>,
    #[serde(deserialize_with = "crate::nullable")]
    block_reason: Option<BlockReason>,
    residency: Residency,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    load_id: Option<LoadId>,
    controller_revision: Revision,
});
impl SessionControlFacts {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if (self.scheduling == Scheduling::Auto) != self.pause_reasons.is_empty() {
            return Err(invalid("scheduling and pause reasons must agree"));
        }
        if self
            .pause_reasons
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != self.pause_reasons.len()
        {
            return Err(invalid("pause reasons must not repeat"));
        }
        if (self.residency == Residency::Unloaded && self.load_id.is_some())
            || (matches!(self.residency, Residency::Loaded | Residency::Unloading)
                && self.load_id.is_none())
        {
            return Err(invalid("load_id must match actual Runner residency"));
        }
        if let Some(reason) = &self.block_reason {
            reason.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionControlKind {
    #[serde(rename = "session.pause")]
    Pause,
    #[serde(rename = "session.resume")]
    Resume,
    #[serde(rename = "session.load")]
    Load,
    #[serde(rename = "session.unload")]
    Unload,
}
crate::validated_dto!(SessionControlReceipt, SessionControlReceiptWire, "SessionControlReceiptWire" {
    kind: SessionControlKind,
    host_id: HostId,
    host_incarnation: HostIncarnation,
    operation_id: OperationId,
    agent_id: AgentId,
    session_id: SessionId,
    result: SessionControlFacts,
});
impl SessionControlReceipt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.result.validate()?;
        if self.kind == SessionControlKind::Pause && self.result.scheduling != Scheduling::Paused {
            Err(invalid(
                "session.pause must return an already-effective pause",
            ))
        } else {
            Ok(())
        }
    }
}

crate::validated_dto!(SessionCancelResult, SessionCancelResultWire, "SessionCancelResultWire" {
    cancel_claimed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    turn_state: Option<TurnState>,
    controller_revision: Revision,
});
impl SessionCancelResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self.turn_state {
            Some(turn_state) => TurnCancelResult {
                cancel_claimed: self.cancel_claimed,
                turn_state,
                controller_revision: self.controller_revision,
            }
            .validate(),
            None if !self.cancel_claimed => Ok(()),
            None => Err(invalid("an idle Session cannot claim a Turn cancellation")),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionCancelKind {
    #[serde(rename = "session.cancel")]
    SessionCancel,
}
crate::validated_dto!(SessionCancelReceipt, SessionCancelReceiptWire, "SessionCancelReceiptWire" {
    kind: SessionCancelKind,
    host_id: HostId,
    host_incarnation: HostIncarnation,
    operation_id: OperationId,
    agent_id: AgentId,
    session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    turn_id: Option<TurnId>,
    result: SessionCancelResult,
});
impl SessionCancelReceipt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.result.validate()?;
        if self.turn_id.is_some() != self.result.turn_state.is_some() {
            Err(invalid(
                "Session cancellation identifies exactly its observed Turn",
            ))
        } else {
            Ok(())
        }
    }
}
