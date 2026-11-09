use minicore_host_protocol::{
    AgentConfig, AgentId, ConfigRevision, HostId, MessageId, MessageMode, MessageSource,
    SessionConfig, SessionId, SessionRef, StorageMode, TurnId, TurnOutcome,
};
use minicore_runtime::{
    LoopId,
    history::{HistoryItem, SummaryHistory, UserHistory},
    model::ModelPartial,
};
use serde::{Deserialize, Serialize};

/// On-disk schema version, independent of the design-specification version.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRecord<C> {
    pub schema_version: u32,
    pub id: HostId,
    pub data: HostData<C>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostData<C> {
    pub config: C,
    pub config_revision: ConfigRevision,
}
impl<C> HostRecord<C> {
    pub fn new(id: HostId, config: C, config_revision: ConfigRevision) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id,
            data: HostData {
                config,
                config_revision,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRecord {
    pub schema_version: u32,
    pub id: AgentId,
    pub host_id: HostId,
    pub data: AgentData,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentData {
    pub name: String,
    pub mode: StorageMode,
    pub config: AgentConfig,
    pub config_revision: ConfigRevision,
    /// Only a reloadable persistent Session may appear here. A runtime default
    /// pointing at a memory Session is represented by None, not a saved pointer.
    pub default_session_id: Option<SessionId>,
}
impl AgentRecord {
    pub fn new(id: AgentId, host_id: HostId, data: AgentData) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id,
            host_id,
            data,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecord {
    pub schema_version: u32,
    pub id: SessionId,
    pub agent_id: AgentId,
    pub data: SessionData,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub mode: StorageMode,
    pub agent_config_revision: ConfigRevision,
    pub config_revision: ConfigRevision,
    pub effective_config: SessionConfig,
    /// Historical creation provenance only, never a resumable delegation.
    pub parent: Option<SessionRef>,
    /// Persist only a committed close, never a runtime closing transition.
    pub closed: Option<SessionClosure>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionClosure {
    pub reason: String,
}
impl SessionRecord {
    pub fn new(id: SessionId, agent_id: AgentId, data: SessionData) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id,
            agent_id,
            data,
        }
    }
}

/// A contiguous Session-local position. It is not a global ID or public cursor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRange {
    pub start: u64,
    pub end: u64,
}
impl SaveRange {
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Native Runtime content is retained exactly. Input provenance is paired with
/// the actual UserHistory instead of duplicating text or fabricating an assistant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum HistoryEntry {
    Input {
        turn_id: TurnId,
        message_id: MessageId,
        source: MessageSource,
        mode: MessageMode,
        in_reply_to: Option<MessageId>,
        user: UserHistory,
    },
    Runtime {
        turn_id: TurnId,
        response_index: u32,
        item: HistoryItem,
    },
    Partial {
        turn_id: TurnId,
        loop_id: LoopId,
        response_index: u32,
        partial: ModelPartial,
    },
    TurnFinished {
        turn_id: TurnId,
        outcome: TurnOutcome,
        reason: Option<String>,
    },
    /// An independent committed compaction need not invent a Turn identity.
    Summary { summary: SummaryHistory },
}
impl HistoryEntry {
    pub fn turn_id(&self) -> Option<&TurnId> {
        match self {
            Self::Input { turn_id, .. }
            | Self::Runtime { turn_id, .. }
            | Self::Partial { turn_id, .. }
            | Self::TurnFinished { turn_id, .. } => Some(turn_id),
            Self::Summary { .. } => None,
        }
    }
    /// A projection, not execution recovery. Partial and finish facts are never
    /// turned into complete Runtime history or inferred tool results.
    pub fn runtime_item(&self) -> Option<HistoryItem> {
        match self {
            Self::Input { user, .. } => Some(HistoryItem::User(user.clone())),
            Self::Runtime { item, .. } => Some(item.clone()),
            Self::Summary { summary } => Some(HistoryItem::Summary(summary.clone())),
            Self::Partial { .. } | Self::TurnFinished { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRecord {
    pub schema_version: u32,
    pub session_id: SessionId,
    pub position: u64,
    pub entry: HistoryEntry,
}
impl HistoryRecord {
    pub fn new(session_id: SessionId, position: u64, entry: HistoryEntry) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            session_id,
            position,
            entry,
        }
    }
}
