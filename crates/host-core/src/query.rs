//! Public query values. Source owners retain the underlying data; these values
//! neither reconstruct previous executions nor add a second retention store.
use crate::{HistoryItemData, MessageBinding, ModelPartialData, TurnStateData};
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnResultData {
    /// The original terminal state, shared with get/list and observations.
    pub info: TurnStateData,
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
