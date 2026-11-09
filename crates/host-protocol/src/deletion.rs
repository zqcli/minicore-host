//! Explicit direct-owner deletion, separate from normal lifecycle controls.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentDeleteScope {
    Empty {},
    Sessions {
        /// Exact direct children, never a directory traversal or scope-tree selector.
        session_ids: Vec<SessionId>,
        /// Explicitly stop the Agent and close these children before deletion.
        #[serde(default)]
        close: bool,
    },
}
impl Default for AgentDeleteScope {
    fn default() -> Self {
        Self::Empty {}
    }
}
impl AgentDeleteScope {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Self::Sessions { session_ids, .. } = self
            && (session_ids.is_empty()
                || session_ids.iter().collect::<BTreeSet<_>>().len() != session_ids.len())
        {
            return Err(invalid(
                "cascade scope requires distinct explicit Session IDs",
            ));
        }
        Ok(())
    }
}
validated_dto!(AgentDeleteRequest, AgentDeleteRequestWire, "AgentDeleteRequestWire" {
    operation_id: OperationId,
    #[serde(default)]
    scope: AgentDeleteScope,
});
impl AgentDeleteRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.scope.validate()
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDeleteRequest {
    pub operation_id: OperationId,
    /// False requires already closed and settled. True authorizes normal close.
    #[serde(default)]
    pub close: bool,
}

/// A failed delete may have removed some files before the reported error.
/// It is neither a retention guarantee nor a successful/rolled-back deletion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataDeletion {
    Pending {},
    Deleted {},
    Reclaimed {},
    NotAttempted {},
    Failed { error: Fault },
}
impl DataDeletion {
    pub fn removed(&self) -> bool {
        matches!(self, Self::Deleted {} | Self::Reclaimed {})
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDeleteResult {
    pub lifecycle: SessionLifecycle,
    pub data: DataDeletion,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedSessionDeleteResult {
    pub session_id: SessionId,
    pub result: SessionDeleteResult,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDeletionState {
    pub scope: AgentDeleteScope,
    pub sessions: Vec<OwnedSessionDeleteResult>,
    pub data: DataDeletion,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDeletionState {
    pub close: bool,
    pub data: DataDeletion,
}
validated_dto!(SessionDeleteAccepted, SessionDeleteAcceptedWire, "SessionDeleteAcceptedWire" {
    lifecycle: SessionCloseLifecycle,
    close: bool,
});
impl SessionDeleteAccepted {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.close && self.lifecycle != SessionCloseLifecycle::Closed {
            return Err(invalid("deletion without close requires closed acceptance"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDeleteLifecycle {
    Retiring,
}
validated_dto!(AgentDeleteAccepted, AgentDeleteAcceptedWire, "AgentDeleteAcceptedWire" {
    lifecycle: AgentDeleteLifecycle,
    scope: AgentDeleteScope,
});
impl AgentDeleteAccepted {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.scope.validate()
    }
}
macro_rules! deletion_receipt {
    ($name:ident, $kind:ident, $variant:ident, $wire:literal, $result:ty $(, $field:ident: $ty:ty)*) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        pub enum $kind {
            #[serde(rename = $wire)]
            $variant,
        }
        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub kind: $kind,
            pub host_id: HostId,
            pub host_incarnation: HostIncarnation,
            pub operation_id: OperationId,
            pub agent_id: AgentId,
            $(pub $field: $ty,)*
            pub result: $result,
        }
    };
}
deletion_receipt!(
    AgentDeleteReceipt,
    AgentDeleteKind,
    AgentDelete,
    "agent.delete",
    AgentDeleteAccepted
);
deletion_receipt!(SessionDeleteReceipt, SessionDeleteKind, SessionDelete, "session.delete", SessionDeleteAccepted, session_id: SessionId);
