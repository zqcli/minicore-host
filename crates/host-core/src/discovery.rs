//! Small discovery values projected by the actual direct owners.
//! Lists contain no effective configuration, history, body, or resource handle.
use crate::{CompactionSource, ToolCallState};
use minicore_host_protocol::*;
use serde::{Deserialize, Serialize};

/// Capabilities assembled for this Host, not permission or version negotiation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub storage_modes: Vec<StorageMode>,
    pub configuration_reload: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostResourceSummary {
    pub config_revision: ConfigRevision,
    pub models: usize,
    pub tools: usize,
}

/// Facts held by the embedding Host. Authentication being enabled does not
/// claim that an external transport listener has been started.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostFacilities {
    pub storage_open: bool,
    pub execution_resources: bool,
    pub network_authentication: bool,
}

/// Exclusive lexical Agent-ID cursor. No limit means all remaining entries.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentListRequest {
    pub cursor: Option<AgentId>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentListPage {
    pub items: Vec<AgentSummary>,
    pub next_cursor: Option<AgentId>,
}

/// Metadata/configuration and a fresh projection of direct Session owners.
/// This value is constructed for a query; it is not retained in Agent state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDetails {
    #[serde(flatten)]
    pub info: crate::AgentInfo,
    pub sessions: Vec<SessionSummary>,
}

/// The Agent's own data-deletion fact, never a copy of its full cascade report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: AgentId,
    pub host_id: HostId,
    pub name: String,
    pub mode: StorageMode,
    pub lifecycle: AgentLifecycle,
    pub config_revision: ConfigRevision,
    pub default_session_id: Option<SessionId>,
    pub metadata_saving: bool,
    pub metadata_fault: Option<Fault>,
    pub deletion: Option<DataDeletion>,
    pub sessions: Vec<SessionSummary>,
}

/// Filters apply before the exclusive cursor and page size. No limit is a
/// request for all matching entries, not a default capacity allowance.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionListRequest {
    pub cursor: Option<SessionId>,
    pub limit: Option<usize>,
    pub mode: Option<StorageMode>,
    pub lifecycle: Option<SessionLifecycle>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionListPage {
    pub items: Vec<SessionSummary>,
    pub next_cursor: Option<SessionId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub agent_id: AgentId,
    pub title: Option<String>,
    pub mode: StorageMode,
    pub config_revision: ConfigRevision,
    pub lifecycle: SessionLifecycle,
    pub scheduling: Scheduling,
    pub pause_reasons: Vec<PauseReason>,
    pub residency: Residency,
    pub active_turn_id: Option<TurnId>,
    pub next_step: usize,
    pub next_turn: usize,
    pub pending_interactions: usize,
    pub owned_work: Box<SessionOwnedWork>,
    pub block_reason: Option<BlockReason>,
    pub metadata_fault: Option<Fault>,
    pub deletion: Option<DataDeletion>,
    pub end_reason: Option<String>,
}

/// Only this Session's current work. Independent child execution is not a
/// running ordinary tool and is never replicated as a Session scheduling tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionOwnedWork {
    /// An actual Runtime model phase, including its owned retry wait.
    pub model: bool,
    pub tools: Vec<OwnedToolWork>,
    pub preparing: bool,
    /// Includes an accepted unconfirmed save after its worker reports a fault.
    pub history_save: bool,
    pub metadata_save: bool,
    pub compaction: Option<OwnedCompactionWork>,
    pub tool_cleanup: bool,
    pub scope_cleanup: bool,
    pub result_handoff: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnedToolWork {
    pub tool_call_id: ToolCallId,
    pub name: String,
    pub state: ToolCallState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnedCompactionWork {
    pub source: CompactionSource,
    pub state: CompactionState,
    pub cancellation_requested: bool,
}

pub(crate) fn validate_list_limit(limit: Option<usize>) -> Result<(), Fault> {
    if limit == Some(0) {
        Err(Fault::new(
            FaultKind::HostObservationInvalid,
            "object list limit must be greater than zero when supplied",
        ))
    } else {
        Ok(())
    }
}
