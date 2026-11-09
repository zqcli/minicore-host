//! Public query values. Source owners retain the underlying data; these values
//! neither reconstruct previous executions nor add a second retention store.
use crate::{HistoryItemData, MessageBinding, ModelPartialData, TurnInfo, TurnStateData};
use minicore_host_protocol::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageData {
    pub text: String,
    pub mode: MessageMode,
    pub source: MessageSource,
    pub in_reply_to: Option<MessageId>,
    pub queue: InputQueue,
    pub order: u64,
    pub binding: MessageBinding,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageSnapshot {
    #[serde(flatten)]
    pub source: SessionIdentity,
    pub message_id: MessageId,
    pub data: MessageData,
}

/// Detailed Turn reads add the original binding to the same lightweight state
/// payload used by observations. State events never carry this configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnDetailsData {
    #[serde(flatten)]
    pub state: TurnStateData,
    pub config: SessionConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnDetailsSnapshot {
    #[serde(flatten)]
    pub source: TurnIdentity,
    pub data: TurnDetailsData,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnListEntry {
    pub turn_id: TurnId,
    pub load_id: LoadId,
    #[serde(flatten)]
    pub data: TurnDetailsData,
}
/// Completion order followed by the active Turn, if any. The actual Session
/// identity appears once, even when the existing result set is empty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnListSnapshot {
    #[serde(flatten)]
    pub source: SessionIdentity,
    pub data: Vec<TurnListEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnHistoryItemData {
    /// The actual public Turn index for complete model and tool records. User
    /// input and summaries do not belong to a model response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_index: Option<u64>,
    #[serde(flatten)]
    pub item: HistoryItemData,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnPartialData {
    pub response_index: u64,
    pub partial: ModelPartialData,
}
/// Immutable terminal facts already retained by the Turn result. Unlike a
/// current state query, this does not borrow the Session's live runner revision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnResultInfo {
    pub state: TurnState,
    pub config_revision: ConfigRevision,
    pub outcome: Option<TurnOutcome>,
    pub persistence: PersistenceState,
    pub requests: u64,
    pub tool_rounds: u64,
    pub phase: Option<TurnPhase>,
    pub model_retry: Option<ModelRetryProgress>,
    pub model_attempts: Vec<u8>,
    pub fault: Option<Fault>,
    pub persistence_fault: Option<Fault>,
    pub cleanup_faults: Vec<Fault>,
}
impl From<&TurnInfo> for TurnResultInfo {
    fn from(info: &TurnInfo) -> Self {
        Self {
            state: info.state,
            config_revision: info.config_revision,
            outcome: info.outcome,
            persistence: info.persistence,
            requests: info.requests,
            tool_rounds: info.tool_rounds,
            phase: info.phase,
            model_retry: info.model_retry.clone(),
            model_attempts: info.model_attempts.clone(),
            fault: info.fault.clone(),
            persistence_fault: info.persistence_fault.clone(),
            cleanup_faults: info.cleanup_faults.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnResultData {
    pub info: TurnResultInfo,
    pub config: SessionConfig,
    /// Complete native records only. Unfinished responses remain in `partials`.
    pub history: Vec<TurnHistoryItemData>,
    pub partials: Vec<TurnPartialData>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnResultSnapshot {
    #[serde(flatten)]
    pub source: TurnIdentity,
    pub data: TurnResultData,
}
