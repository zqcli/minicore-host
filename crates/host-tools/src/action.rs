use crate::{AgentChanges, SessionChanges};
use minicore_host_protocol::*;
use serde::{Deserialize, Deserializer, Serialize};

/// Six actions, with distinct create/update/control branches. No generic RPC
/// forwarding, caller identity, operation ID, or model-supplied revision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentAction {
    Create(CreateAction),
    Send(SendAction),
    Inspect(InspectAction),
    Wait(WaitAction),
    Control(ControlAction),
    Update(UpdateAction),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostTarget {
    pub host_id: HostId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "resource", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreateAction {
    Agent {
        target: HostTarget,
        name: String,
        mode: StorageMode,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        from_agent_id: Option<AgentId>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        settings: Option<SessionConfigPatch>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        text: Option<String>,
    },
    Session {
        target: AgentRef,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        mode: Option<StorageMode>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        settings: Option<SessionConfigPatch>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        text: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendAction {
    pub target: MessageTarget,
    pub text: String,
    #[serde(default = "follow_up")]
    pub mode: MessageMode,
    #[serde(default)]
    pub delegate: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub in_reply_to: Option<MessageId>,
}
fn follow_up() -> MessageMode {
    MessageMode::FollowUp
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InspectTarget {
    Agent {
        host_id: HostId,
        agent_id: AgentId,
    },
    Session {
        agent_id: AgentId,
        session_id: SessionId,
    },
    Turn {
        agent_id: AgentId,
        session_id: SessionId,
        turn_id: TurnId,
    },
    Toolcall {
        agent_id: AgentId,
        session_id: SessionId,
        turn_id: TurnId,
        tool_call_id: ToolCallId,
    },
    Message {
        agent_id: AgentId,
        session_id: SessionId,
        message_id: MessageId,
    },
    Receipt {
        host_id: HostId,
        operation_id: OperationId,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectAction {
    pub target: InspectTarget,
    #[serde(default)]
    pub include_output: bool,
    #[serde(default)]
    pub include_config: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitAction {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlAction {
    CancelTurn {
        target: SessionRef,
        turn_id: TurnId,
        #[serde(default)]
        keep_inbox: bool,
    },
    CancelSession {
        target: SessionRef,
        #[serde(default)]
        keep_inbox: bool,
    },
    CloseSession {
        target: SessionRef,
    },
    CloseScope {},
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "resource", rename_all = "snake_case", deny_unknown_fields)]
pub enum UpdateAction {
    Agent {
        target: AgentRef,
        changes: AgentChanges,
    },
    Session {
        target: SessionRef,
        changes: SessionChanges,
    },
}

impl AgentAction {
    pub fn parse(arguments: serde_json::Value) -> Result<Self, Fault> {
        let action: Self =
            serde_json::from_value(arguments).map_err(|error| invalid(error.to_string()))?;
        action.validate()?;
        Ok(action)
    }
    pub fn validate(&self) -> Result<(), Fault> {
        match self {
            Self::Create(CreateAction::Agent { name, settings, .. }) => {
                validate_name(name)?;
                validate_settings(settings.as_ref())
            }
            Self::Create(CreateAction::Session { settings, .. }) => {
                validate_settings(settings.as_ref())
            }
            Self::Send(request)
                if request.delegate && !matches!(request.target, MessageTarget::Session { .. }) =>
            {
                Err(invalid("delegate:true requires an explicit Session target"))
            }
            Self::Inspect(request) => request.validate(),
            Self::Update(UpdateAction::Agent { changes, .. }) => changes.validate(),
            Self::Update(UpdateAction::Session { changes, .. }) => changes.validate(),
            _ => Ok(()),
        }
    }
}
impl InspectAction {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.include_config
            && !matches!(
                self.target,
                InspectTarget::Agent { .. } | InspectTarget::Session { .. }
            )
        {
            return Err(invalid("include_config is only valid for Agent or Session"));
        }
        if self.include_output
            && !matches!(
                self.target,
                InspectTarget::Session { .. }
                    | InspectTarget::Turn { .. }
                    | InspectTarget::Toolcall { .. }
                    | InspectTarget::Message { .. }
            )
        {
            return Err(invalid(
                "include_output requires Session, Turn, ToolCall or Message",
            ));
        }
        Ok(())
    }
}
fn validate_settings(settings: Option<&SessionConfigPatch>) -> Result<(), Fault> {
    if let Some(settings) = settings {
        SessionConfig::default()
            .apply_patch(settings)
            .map_err(|error| invalid(error.to_string()))?;
    }
    Ok(())
}
pub(crate) fn validate_name(value: &str) -> Result<(), Fault> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(invalid("name/title must be nonblank and contain no NUL"));
    }
    Ok(())
}
pub(crate) fn invalid(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::SessionInputInvalid, message.into())
}
pub(crate) fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
