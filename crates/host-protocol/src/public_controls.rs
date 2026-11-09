//! Explicit metadata controls and the two owner-held input queues.
use crate::*;
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedInputRecord {
    pub input: MessageInput,
    pub source: MessageSource,
    pub queue: InputQueue,
    pub order: u64,
}
validated_dto!(InputList, InputListWire, "InputListWire" {
    host_id: HostId,
    host_incarnation: HostIncarnation,
    agent_id: AgentId,
    session_id: SessionId,
    controller_revision: ControllerRevision,
    next_step: Vec<QueuedInputRecord>,
    next_turn: Vec<QueuedInputRecord>,
});
impl InputList {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (queue, entries) in [
            (InputQueue::NextStep, &self.next_step),
            (InputQueue::NextTurn, &self.next_turn),
        ] {
            if entries.iter().any(|entry| entry.queue != queue)
                || entries
                    .windows(2)
                    .any(|pair| pair[0].order >= pair[1].order)
            {
                return Err(invalid(
                    "input entries must retain their queue and increasing queue order",
                ));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRemoveRequest {
    pub operation_id: OperationId,
    pub message_id: MessageId,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub expected_controller_revision: Option<ControllerRevision>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRemoveResult {
    pub queue: InputQueue,
    pub order: u64,
    pub controller_revision: ControllerRevision,
}

macro_rules! control_receipt {
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
control_receipt!(
    AgentUpdateReceipt,
    AgentUpdateKind,
    AgentUpdate,
    "agent.update",
    MetadataUpdateResult
);
control_receipt!(SessionUpdateReceipt, SessionUpdateKind, SessionUpdate, "session.update", MetadataUpdateResult, session_id: SessionId);
control_receipt!(
    AgentStartReceipt,
    AgentStartKind,
    AgentStart,
    "agent.start",
    AgentStartResult
);
control_receipt!(
    AgentSetDefaultSessionReceipt,
    AgentSetDefaultSessionKind,
    AgentSetDefaultSession,
    "agent.set_default_session",
    AgentSetDefaultSessionResult
);
control_receipt!(InputRemoveReceipt, InputRemoveKind, InputRemove, "input.remove", InputRemoveResult, session_id: SessionId, message_id: MessageId);
