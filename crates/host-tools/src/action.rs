use crate::{AgentChanges, AgentSettingsPatch, SessionChanges};
use minicore_host_protocol::*;
use serde::{Deserialize, Deserializer, Serialize};
use std::num::NonZeroUsize;

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
        /// Omitted: this Host.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        target: Option<HostTarget>,
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
        settings: Option<AgentSettingsPatch>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        text: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        allow_delegation: Option<bool>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        max_children: Option<NonZeroUsize>,
    },
    Session {
        /// Omitted: the calling Agent.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        target: Option<AgentRef>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        name: Option<String>,
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
        settings: Option<AgentSettingsPatch>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        text: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        allow_delegation: Option<bool>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        max_children: Option<NonZeroUsize>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendAction {
    pub target: MessageTarget,
    pub text: String,
    #[serde(default = "steer")]
    pub mode: MessageMode,
    #[serde(default)]
    pub delegate: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub allow_delegation: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub max_children: Option<NonZeroUsize>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub in_reply_to: Option<MessageId>,
}
fn steer() -> MessageMode {
    MessageMode::Steer
}

pub(crate) const DEFAULT_MAX_CHILDREN: NonZeroUsize = NonZeroUsize::new(2).unwrap();

/// `None` means the caller did not specify a grant. An explicit `false` still
/// cannot modify a registered relationship on an append.
#[derive(Clone, Copy)]
pub(crate) enum DelegationGrant {
    Disabled,
    Allowed(NonZeroUsize),
}
impl DelegationGrant {
    pub(crate) fn requested(
        allow_delegation: Option<bool>,
        max_children: Option<NonZeroUsize>,
    ) -> Option<Self> {
        allow_delegation.map(|allow| {
            if allow {
                Self::Allowed(max_children.unwrap_or(DEFAULT_MAX_CHILDREN))
            } else {
                Self::Disabled
            }
        })
    }
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
    /// Omitted: the calling Session.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub target: Option<InspectTarget>,
    #[serde(default)]
    pub include_output: bool,
    #[serde(default)]
    pub include_config: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub output_range: Option<TextReadRequest>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub response_index: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub history_page: Option<HistoryReadRequest>,
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
    pub fn parse(mut arguments: serde_json::Value) -> Result<Self, Fault> {
        if arguments.get("action").and_then(|value| value.as_str()) == Some("create")
            && arguments.get("resource").is_none()
        {
            if arguments
                .get("text")
                .and_then(|value| value.as_str())
                .is_none_or(|text| text.trim().is_empty())
            {
                return Err(invalid("create without resource requires nonblank text"));
            }
            arguments["resource"] = serde_json::json!("session");
        }
        let accepted = accepted_fields(&arguments);
        let action: Self = serde_json::from_value(arguments).map_err(|error| {
            let fault = parse_error(error);
            match &accepted {
                Some(fields) if fault.message() == UNKNOWN_FIELD => {
                    let mut message = format!("{UNKNOWN_FIELD}; this action accepts {fields}");
                    if fields.contains("`settings`") {
                        message.push_str(
                            "; model, reasoning, instructions, role, tools, workspace and execution go inside settings",
                        );
                    }
                    invalid(message)
                }
                _ => fault,
            }
        })?;
        action.validate()?;
        Ok(action)
    }
    pub fn validate(&self) -> Result<(), Fault> {
        match self {
            Self::Create(CreateAction::Agent {
                name,
                settings,
                text,
                allow_delegation,
                max_children,
                ..
            }) => {
                validate_create_grant(text.as_deref(), *allow_delegation, *max_children)?;
                validate_name(name)?;
                validate_settings(settings.as_ref())
            }
            Self::Create(CreateAction::Session {
                name,
                settings,
                text,
                allow_delegation,
                max_children,
                ..
            }) => {
                validate_create_grant(text.as_deref(), *allow_delegation, *max_children)?;
                if let Some(name) = name {
                    validate_name(name)?;
                }
                validate_settings(settings.as_ref())
            }
            Self::Send(request) => {
                if request.delegate && !matches!(request.target, MessageTarget::Session { .. }) {
                    return Err(invalid("delegate:true requires an explicit Session target"));
                }
                if request.allow_delegation.is_some() && !request.delegate {
                    return Err(invalid("allow_delegation on send requires delegate:true"));
                }
                validate_child_limit(request.allow_delegation, request.max_children)
            }
            Self::Inspect(request) => request.validate(),
            Self::Update(UpdateAction::Agent { changes, .. }) => changes.validate(),
            Self::Update(UpdateAction::Session { changes, .. }) => changes.validate(),
            _ => Ok(()),
        }
    }
}
fn validate_create_grant(
    text: Option<&str>,
    allow_delegation: Option<bool>,
    max_children: Option<NonZeroUsize>,
) -> Result<(), Fault> {
    if allow_delegation.is_some() && text.is_none_or(|text| text.trim().is_empty()) {
        return Err(invalid("allow_delegation on create requires nonblank text"));
    }
    validate_child_limit(allow_delegation, max_children)
}
fn validate_child_limit(
    allow_delegation: Option<bool>,
    max_children: Option<NonZeroUsize>,
) -> Result<(), Fault> {
    if max_children.is_some() && allow_delegation != Some(true) {
        return Err(invalid("max_children requires allow_delegation:true"));
    }
    Ok(())
}
const UNKNOWN_FIELD: &str = "arguments contain an unknown field";

/// The field names this call's own schema branch accepts, chosen by its
/// action/resource/operation constants. Only schema text is returned, never
/// argument text, so a forged field name cannot shape the diagnostic.
fn accepted_fields(arguments: &serde_json::Value) -> Option<String> {
    let spec = crate::agent_tool_spec();
    let branches = spec.input_schema().get("oneOf")?.as_array()?;
    let branch = branches.iter().find(|branch| {
        ["action", "resource", "operation"].iter().all(|key| {
            match branch["properties"]
                .get(*key)
                .and_then(|value| value.get("const"))
            {
                Some(expected) => arguments.get(*key) == Some(expected),
                None => true,
            }
        })
    })?;
    let names: Vec<String> = branch["properties"]
        .as_object()?
        .keys()
        .map(|name| format!("`{name}`"))
        .collect();
    Some(names.join(", "))
}

fn parse_error(error: serde_json::Error) -> Fault {
    let message = error.to_string();
    // Unknown names may contain arbitrary argument text, and an empty struct
    // does not supply an expected-field suffix. Never parse those names back
    // out of Serde's display message.
    for (prefix, reason) in [
        ("unknown field ", UNKNOWN_FIELD),
        (
            "unknown variant ",
            "arguments contain an unsupported variant",
        ),
    ] {
        if message.starts_with(prefix) {
            return invalid(reason);
        }
    }
    // These Serde diagnostics always end with the expectation from our DTO
    // types. The supplied actual value must not appear in the public error.
    for (prefix, reason) in [
        ("invalid type: ", "arguments contain an invalid type"),
        ("invalid value: ", "arguments contain an invalid value"),
        ("invalid length ", "arguments contain an invalid length"),
    ] {
        if message.starts_with(prefix) {
            return invalid(match message.rsplit_once(", expected ") {
                Some((_, expected)) => format!("{reason}; expected {expected}"),
                None => reason.to_owned(),
            });
        }
    }
    // The remaining custom deserializers use fixed local validation messages,
    // including ID grammar, references and execution-setting constraints.
    invalid(message)
}
impl InspectAction {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.include_config
            && !matches!(
                self.target,
                None | Some(InspectTarget::Agent { .. } | InspectTarget::Session { .. })
            )
        {
            return Err(invalid("include_config is only valid for Agent or Session"));
        }
        if self.include_output
            && !matches!(
                self.target,
                None | Some(
                    InspectTarget::Session { .. }
                        | InspectTarget::Turn { .. }
                        | InspectTarget::Toolcall { .. }
                        | InspectTarget::Message { .. }
                )
            )
        {
            return Err(invalid(
                "include_output requires Session, Turn, ToolCall or Message",
            ));
        }
        if let Some(range) = &self.output_range {
            if !self.include_output
                || !matches!(
                    self.target,
                    Some(InspectTarget::Turn { .. } | InspectTarget::Toolcall { .. })
                )
            {
                return Err(invalid(
                    "output_range requires Turn or ToolCall with include_output:true",
                ));
            }
            range
                .validate()
                .map_err(|error| invalid(error.to_string()))?;
            if matches!(self.target, Some(InspectTarget::Turn { .. }))
                && self.response_index.is_none()
            {
                return Err(invalid("Turn output_range requires response_index"));
            }
        }
        if self.response_index.is_some()
            && (self.output_range.is_none()
                || !matches!(self.target, Some(InspectTarget::Turn { .. })))
        {
            return Err(invalid(
                "response_index is only valid with a Turn output_range",
            ));
        }
        if let Some(page) = &self.history_page {
            if !self.include_output
                || !matches!(self.target, None | Some(InspectTarget::Session { .. }))
            {
                return Err(invalid(
                    "history_page requires Session with include_output:true",
                ));
            }
            page.validate()
                .map_err(|error| invalid(error.to_string()))?;
        }
        Ok(())
    }
}
fn validate_settings(settings: Option<&AgentSettingsPatch>) -> Result<(), Fault> {
    if let Some(settings) = settings {
        let base = SessionConfig::default();
        let patch = settings
            .resolve(&base.execution)
            .map_err(|error| invalid(error.to_string()))?;
        base.apply_patch(&patch)
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
