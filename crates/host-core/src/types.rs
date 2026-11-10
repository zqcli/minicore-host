use minicore_host_protocol::*;
use minicore_runtime::{HistoryItem, model::ModelPartial};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostInfo {
    pub name: String,
    pub version: String,
    pub capabilities: crate::HostCapabilities,
    pub resources: crate::HostResourceSummary,
    pub facilities: crate::HostFacilities,
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub lifecycle: HostLifecycle,
    pub shutdown_strategy: Option<HostShutdownStrategy>,
    pub shutdown_fault: Option<Fault>,
    pub control: ControlStateData,
    /// Read-only health of the shared process pipeline, not per-Host counters.
    pub logging: minicore_host_diagnostics::LoggingStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion: Option<AgentDeletionState>,
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
    pub owned_work: Box<crate::SessionOwnedWork>,
    pub end_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion: Option<SessionDeletionState>,
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
    pub compaction: Option<Box<CompactionActivity>>,
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
    /// The original Session configuration accepted with this Turn's binding.
    pub config: SessionConfig,
    pub outcome: Option<TurnOutcome>,
    pub persistence: PersistenceState,
    pub requests: u64,
    pub tool_rounds: u64,
    pub phase: Option<TurnPhase>,
    pub model_retry: Option<ModelRetryProgress>,
    pub model_attempts: Vec<u8>,
    pub fault: Option<Fault>,
    pub persistence_fault: Option<Fault>,
    /// Cleanup diagnostics never replace the claimed business outcome/fault.
    #[serde(default)]
    pub cleanup_faults: Vec<Fault>,
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

/// Name-only convenience for embedding construction. Persistent storage always
/// uses the complete HostConfig shape, never a second settings record format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostSettings {
    pub name: String,
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
        /// Absent only when the native history has no original Host input facts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<MessageSource>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<MessageMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        in_reply_to: Option<MessageId>,
    },
    Assistant {
        response: crate::ModelResponseData,
    },
    Partial {
        partial: ModelPartialData,
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

/// Unfinished model content, never a complete response or executable context.
/// Like complete response reads, this excludes internal request/replay metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelPartialData {
    pub content: Vec<minicore_runtime::model::AssistantPart>,
    pub incomplete_tool_calls: Vec<minicore_runtime::model::IncompleteToolCall>,
    pub usage: Option<minicore_runtime::model::Usage>,
}
impl From<&ModelPartial> for ModelPartialData {
    fn from(value: &ModelPartial) -> Self {
        Self {
            content: value.content.clone(),
            incomplete_tool_calls: value.incomplete_tool_calls.clone(),
            usage: value.usage,
        }
    }
}
impl From<&HistoryItem> for HistoryItemData {
    fn from(value: &HistoryItem) -> Self {
        match value {
            HistoryItem::User(value) => Self::User {
                text: value.input.as_text().to_owned(),
                message_id: None,
                source: None,
                mode: None,
                in_reply_to: None,
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

/// Utility calls are accounted separately from ordinary Turn responses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionUsage {
    pub request_index: u32,
    pub usage: Option<minicore_runtime::model::Usage>,
    pub completed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactionSource {
    Manual {
        operation_id: OperationId,
    },
    Automatic {
        turn_id: TurnId,
        load_id: LoadId,
        /// The pending ordinary request's continuous public Turn index. Utility
        /// requests retain their separate indexes in `utility_usage`.
        response_index: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionActivity {
    pub source: CompactionSource,
    pub state: CompactionState,
    pub cancellation_requested: bool,
    pub utility_usage: Vec<CompactionUsage>,
}
