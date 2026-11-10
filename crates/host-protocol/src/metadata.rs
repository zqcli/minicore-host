//! Atomic metadata and configuration updates for Agent and Session owners.
use crate::{
    AgentConfigPatch, AgentId, ConfigRevision, NullablePatch, OperationId, SessionConfigPatch,
    SessionId, ValidationError, control_receipt, invalid, validated_dto,
};
use serde::{Deserialize, Serialize};

validated_dto!(#[derive(Default)] AgentUpdateChanges, AgentUpdateChangesWire, "AgentUpdateChangesWire" {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    name: Option<String>,
    #[serde(default)]
    config: AgentConfigPatch,
});
impl AgentUpdateChanges {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_none() && self.config.is_empty() {
            return Err(invalid("Agent changes must include an allowed field"));
        }
        if let Some(name) = &self.name {
            validate_label(name)?;
        }
        Ok(())
    }
}
validated_dto!(#[derive(Default)] SessionUpdateChanges, SessionUpdateChangesWire, "SessionUpdateChangesWire" {
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    title: NullablePatch<String>,
    #[serde(default)]
    config: SessionConfigPatch,
});
impl SessionUpdateChanges {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.title.is_unchanged() && self.config.is_empty() {
            return Err(invalid("Session changes must include an allowed field"));
        }
        if let NullablePatch::Set(title) = &self.title {
            validate_label(title)?;
        }
        Ok(())
    }
}
fn validate_label(value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.contains('\0') {
        Err(invalid("name/title must be nonblank and contain no NUL"))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentUpdateRequest {
    pub operation_id: OperationId,
    pub expected_config_revision: ConfigRevision,
    pub changes: AgentUpdateChanges,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionUpdateRequest {
    pub operation_id: OperationId,
    pub expected_config_revision: ConfigRevision,
    pub changes: SessionUpdateChanges,
}

/// Saving reports the still-committed revision. A receipt never changes when
/// the owner later commits or reports its metadata fault through get/state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetadataUpdateResult {
    Saving { config_revision: ConfigRevision },
    Committed { config_revision: ConfigRevision },
}

control_receipt!(
    AgentUpdateReceipt,
    AgentUpdateKind,
    AgentUpdate,
    "agent.update",
    MetadataUpdateResult,
    agent_id: AgentId
);
control_receipt!(SessionUpdateReceipt, SessionUpdateKind, SessionUpdate, "session.update", MetadataUpdateResult, agent_id: AgentId, session_id: SessionId);
