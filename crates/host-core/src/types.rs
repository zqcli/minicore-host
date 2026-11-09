use minicore_host_protocol::*;
use minicore_runtime::{HistoryItem, model::ModelPartial};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostInfo {
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub lifecycle: HostLifecycle,
    pub shutdown_strategy: Option<HostShutdownStrategy>,
    pub shutdown_fault: Option<Fault>,
    pub control: ControlStateData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: AgentId,
    pub host_id: HostId,
    pub name: String,
    pub mode: StorageMode,
    pub lifecycle: AgentLifecycle,
    pub config_revision: ConfigRevision,
    pub config: AgentConfig,
    pub default_session_id: Option<SessionId>,
    pub metadata_saving: bool,
    pub metadata_fault: Option<Fault>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub agent_config_revision: ConfigRevision,
    pub id: SessionId,
    pub agent_id: AgentId,
    pub title: Option<String>,
    pub mode: StorageMode,
    pub config_revision: ConfigRevision,
    pub config: SessionConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputSummary {
    pub message_id: MessageId,
    pub mode: MessageMode,
    pub queue: InputQueue,
    pub order: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MessageBinding {
    Queued,
    Bound { turn_id: TurnId },
    Removed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageRecord {
    pub input: MessageInput,
    pub source: MessageSource,
    pub queue: InputQueue,
    pub order: u64,
    pub binding: MessageBinding,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionStateData {
    pub mode: StorageMode,
    pub controller_revision: ControllerRevision,
    pub lifecycle: SessionLifecycle,
    pub scheduling: Scheduling,
    pub pause_reasons: Vec<PauseReason>,
    pub residency: Residency,
    pub load_id: Option<LoadId>,
    pub runner_revision: u64,
    pub active_turn_id: Option<TurnId>,
    pub pending_interactions: Vec<crate::PendingInteractionData>,
    pub next_step: Vec<InputSummary>,
    pub next_turn: Vec<InputSummary>,
    pub preparing: bool,
    pub saving: bool,
    pub metadata_saving: bool,
    pub metadata_fault: Option<Fault>,
    pub block_reason: Option<BlockReason>,
    pub last_result: Option<TurnRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub data: SessionStateData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnInfo {
    pub id: TurnId,
    pub session_id: SessionId,
    pub load_id: LoadId,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialResult {
    pub response_index: u64,
    pub partial: ModelPartial,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnResult {
    pub info: TurnInfo,
    pub history: Vec<HistoryItem>,
    pub partials: Vec<PartialResult>,
}

/// Controller cancellation outcome. Only `Claimed` authorizes the optional
/// queue clear. Repeated or stale cancellation is a factual no-op.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelStatus {
    Claimed,
    AlreadyEnding,
    NotActive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CancelResult {
    pub status: CancelStatus,
    pub cleared: Vec<MessageId>,
}

/// Only the identity metadata supported by this stage. This is deliberately not
/// a substitute for provider, tool, network, credential, or logging settings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSettings {
    pub name: String,
}
impl HostSettings {
    pub(crate) fn validate(&self) -> Result<(), Fault> {
        if self.name.trim().is_empty() {
            return Err(Fault::new(
                FaultKind::HostStorageUnavailable,
                "Host name must not be blank",
            ));
        }
        Ok(())
    }
}

/// A discovery failure remains associated with its actual direct child. It is
/// not converted to an empty object and does not prevent healthy siblings loading.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentLoadFailure {
    pub agent_id: AgentId,
    pub fault: Fault,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionLoadFailure {
    pub session_id: SessionId,
    pub fault: Fault,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelTextSnapshot {
    #[serde(flatten)]
    pub source: ModelOutputIdentity,
    pub data: TextRangeData,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolTextSnapshot {
    #[serde(flatten)]
    pub source: ToolCallIdentity,
    pub data: TextRangeData,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallSnapshot {
    #[serde(flatten)]
    pub source: ToolCallIdentity,
    pub data: crate::ToolCallData,
}

/// Public conversation content, not a raw JSONL record or a reconstructed past
/// execution. Restarted histories do not acquire fabricated current Turn/Load IDs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryItemData {
    User {
        text: String,
    },
    Assistant {
        response: crate::ModelResponseData,
    },
    ToolResult {
        tool_call_id: ToolCallId,
        name: String,
        result: crate::ToolResultData,
    },
    Summary {
        text: String,
    },
}
impl From<&HistoryItem> for HistoryItemData {
    fn from(value: &HistoryItem) -> Self {
        match value {
            HistoryItem::User(value) => Self::User {
                text: value.input.as_text().to_owned(),
            },
            HistoryItem::Assistant(value) => Self::Assistant {
                response: value.into(),
            },
            HistoryItem::ToolResult(value) => Self::ToolResult {
                tool_call_id: ToolCallId(value.call_id.as_str().to_owned()),
                name: value.tool_name.as_str().to_owned(),
                result: value.into(),
            },
            HistoryItem::Summary(value) => Self::Summary {
                text: value.content.as_str().to_owned(),
            },
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionHistorySnapshot {
    #[serde(flatten)]
    pub source: SessionIdentity,
    pub data: HistoryPageData<HistoryItemData>,
}
