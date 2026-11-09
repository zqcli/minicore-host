use crate::{
    AgentConfig, AgentId, AgentRef, Fault, HostId, HostIncarnation, LoadId, MessageId, OperationId,
    Revision, SessionId, SessionOverrides, SessionRef, StorageMode, ValidationError, invalid,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageMode {
    FollowUp,
    Steer,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InputQueue {
    #[serde(rename = "next-step")]
    NextStep,
    #[serde(rename = "next-turn")]
    NextTurn,
}
impl MessageMode {
    /// Requested scheduling only. The Session may choose next-turn when its
    /// active Turn is already cancelling/finalizing; receipts retain that fact.
    pub const fn requested_queue(self) -> InputQueue {
        match self {
            Self::FollowUp => InputQueue::NextTurn,
            Self::Steer => InputQueue::NextStep,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageTarget {
    AgentDefault {
        host_id: HostId,
        agent_id: AgentId,
    },
    Session {
        agent_id: AgentId,
        session_id: SessionId,
    },
}
impl From<AgentRef> for MessageTarget {
    fn from(value: AgentRef) -> Self {
        Self::AgentDefault {
            host_id: value.host_id,
            agent_id: value.agent_id,
        }
    }
}
impl From<SessionRef> for MessageTarget {
    fn from(value: SessionRef) -> Self {
        Self::Session {
            agent_id: value.agent_id,
            session_id: value.session_id,
        }
    }
}

/// A public message is text only. Paths are text, not Host-readable attachments.
/// Sources, control rights, waiting options, and ToolResults are not accepted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageInput {
    pub message_id: MessageId,
    pub mode: MessageMode,
    pub text: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub in_reply_to: Option<MessageId>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessage {
    pub message_id: MessageId,
    pub mode: MessageMode,
    pub text: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub in_reply_to: Option<MessageId>,
    pub target: MessageTarget,
}
impl SendMessage {
    pub fn new(input: MessageInput, target: impl Into<MessageTarget>) -> Self {
        Self {
            message_id: input.message_id,
            mode: input.mode,
            text: input.text,
            in_reply_to: input.in_reply_to,
            target: target.into(),
        }
    }
    pub fn input(&self) -> MessageInput {
        MessageInput {
            message_id: self.message_id.clone(),
            mode: self.mode,
            text: self.text.clone(),
            in_reply_to: self.in_reply_to.clone(),
        }
    }
}

/// Historical provenance only, populated by a trusted owner at acceptance.
/// Deserializing persisted provenance never grants live internal authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageSource {
    User,
    Agent { source: SessionRef },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Delivery {
    #[serde(rename = "accepted")]
    Accepted,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MessageSendKind {
    #[serde(rename = "message.send")]
    MessageSend,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageAcceptedFacts {
    pub delivery: Delivery,
    pub queue: InputQueue,
    pub controller_revision: Revision,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub load_id: Option<LoadId>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub start_error: Option<Fault>,
}
impl MessageAcceptedFacts {
    pub fn new(queue: InputQueue, controller_revision: Revision) -> Self {
        Self {
            delivery: Delivery::Accepted,
            queue,
            controller_revision,
            load_id: None,
            start_error: None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageAccepted {
    pub kind: MessageSendKind,
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub message_id: MessageId,
    pub result: MessageAcceptedFacts,
}
impl MessageAccepted {
    pub fn new(
        host_id: HostId,
        host_incarnation: HostIncarnation,
        receiver: SessionRef,
        message_id: MessageId,
        result: MessageAcceptedFacts,
    ) -> Self {
        Self {
            kind: MessageSendKind::MessageSend,
            host_id,
            host_incarnation,
            agent_id: receiver.agent_id,
            session_id: receiver.session_id,
            message_id,
            result,
        }
    }
    pub fn receiver(&self) -> SessionRef {
        SessionRef {
            agent_id: self.agent_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOptions {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub mode: Option<StorageMode>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub overrides: Option<SessionOverrides>,
}

crate::validated_dto!(AgentCreate, AgentCreateWire, "AgentCreateWire" {
    operation_id: OperationId,
    name: String,
    mode: StorageMode,
    config: AgentConfig,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    default_session: Option<SessionOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    initial_message: Option<MessageInput>,
});
impl AgentCreate {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(invalid("Agent name must be nonblank"));
        }
        self.config.validate()?;
        let options = self.default_session.clone().unwrap_or_default();
        self.mode.session_mode(options.mode)?;
        crate::SessionConfig::from_agent(&self.config, &options.overrides.unwrap_or_default())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCreate {
    pub operation_id: OperationId,
    pub target: AgentRef,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub mode: Option<StorageMode>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub overrides: Option<SessionOverrides>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub initial_message: Option<MessageInput>,
}
impl SessionCreate {
    /// The actual direct Agent supplies its mode, never the requester.
    pub fn resolved_mode(&self, owner_mode: StorageMode) -> Result<StorageMode, ValidationError> {
        owner_mode.session_mode(self.mode)
    }
}
