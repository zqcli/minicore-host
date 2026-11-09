use crate::{
    AgentId, Delivery, Fault, HostId, HostIncarnation, InputQueue, LoadId, MessageAcceptedFacts,
    MessageId, OperationId, Revision, SessionId, ValidationError, invalid,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CreateStep {
    Pending,
    Running,
    Succeeded,
    NotAttempted,
    Failed { error: Fault },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CreateStatus {
    Pending,
    Running,
    Succeeded,
    NotAttempted,
    Failed,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateStepWire {
    status: CreateStatus,
    #[serde(default, deserialize_with = "crate::present")]
    error: Option<Fault>,
}
impl<'de> Deserialize<'de> for CreateStep {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = CreateStepWire::deserialize(d)?;
        match (wire.status, wire.error) {
            (CreateStatus::Pending, None) => Ok(Self::Pending),
            (CreateStatus::Running, None) => Ok(Self::Running),
            (CreateStatus::Succeeded, None) => Ok(Self::Succeeded),
            (CreateStatus::NotAttempted, None) => Ok(Self::NotAttempted),
            (CreateStatus::Failed, Some(error)) => Ok(Self::Failed { error }),
            _ => Err(serde::de::Error::custom(
                "exactly the failed creation step must carry an error",
            )),
        }
    }
}
impl CreateStep {
    pub fn succeeded(&self) -> bool {
        matches!(self, Self::Succeeded)
    }
    pub fn in_progress(&self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
    fn prevents_following(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::NotAttempted)
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCreateSteps {
    pub agent: CreateStep,
    pub session: CreateStep,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub message: Option<CreateStep>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCreateSteps {
    pub session: CreateStep,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub message: Option<CreateStep>,
}

crate::validated_dto!(AgentCreateResult, AgentCreateResultWire, "AgentCreateResultWire" {
    steps: AgentCreateSteps,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    delivery: Option<Delivery>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    queue: Option<InputQueue>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    controller_revision: Option<Revision>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    load_id: Option<LoadId>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    start_error: Option<Fault>,
});
crate::validated_dto!(SessionCreateResult, SessionCreateResultWire, "SessionCreateResultWire" {
    steps: SessionCreateSteps,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    delivery: Option<Delivery>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    queue: Option<InputQueue>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    controller_revision: Option<Revision>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    load_id: Option<LoadId>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    start_error: Option<Fault>,
});

impl AgentCreateResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.steps.agent == CreateStep::NotAttempted {
            return Err(invalid(
                "the primary Agent creation step cannot be not_attempted",
            ));
        }
        validate_order(&self.steps.agent, &self.steps.session)?;
        if let Some(message) = &self.steps.message {
            validate_order(&self.steps.session, message)?;
        }
        validate_delivery(
            self.steps.message.as_ref(),
            self.delivery,
            self.queue,
            self.controller_revision,
            self.load_id.is_some(),
            self.start_error.is_some(),
        )
    }
    pub fn without_delivery(steps: AgentCreateSteps) -> Self {
        Self {
            steps,
            delivery: None,
            queue: None,
            controller_revision: None,
            load_id: None,
            start_error: None,
        }
    }
    pub fn accepted(
        steps: AgentCreateSteps,
        facts: MessageAcceptedFacts,
    ) -> Result<Self, ValidationError> {
        let result = Self {
            steps,
            delivery: Some(facts.delivery),
            queue: Some(facts.queue),
            controller_revision: Some(facts.controller_revision),
            load_id: facts.load_id,
            start_error: facts.start_error,
        };
        result.validate()?;
        Ok(result)
    }
}
impl SessionCreateResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.steps.session == CreateStep::NotAttempted {
            return Err(invalid(
                "the primary Session creation step cannot be not_attempted",
            ));
        }
        if let Some(message) = &self.steps.message {
            validate_order(&self.steps.session, message)?;
        }
        validate_delivery(
            self.steps.message.as_ref(),
            self.delivery,
            self.queue,
            self.controller_revision,
            self.load_id.is_some(),
            self.start_error.is_some(),
        )
    }
    pub fn without_delivery(steps: SessionCreateSteps) -> Self {
        Self {
            steps,
            delivery: None,
            queue: None,
            controller_revision: None,
            load_id: None,
            start_error: None,
        }
    }
    pub fn accepted(
        steps: SessionCreateSteps,
        facts: MessageAcceptedFacts,
    ) -> Result<Self, ValidationError> {
        let result = Self {
            steps,
            delivery: Some(facts.delivery),
            queue: Some(facts.queue),
            controller_revision: Some(facts.controller_revision),
            load_id: facts.load_id,
            start_error: facts.start_error,
        };
        result.validate()?;
        Ok(result)
    }
}

fn validate_order(previous: &CreateStep, next: &CreateStep) -> Result<(), ValidationError> {
    if previous.prevents_following() && *next != CreateStep::NotAttempted {
        return Err(invalid(
            "a failed prerequisite requires the next step to be not_attempted",
        ));
    }
    if previous.in_progress() && *next != CreateStep::Pending {
        return Err(invalid(
            "a step waiting for an unfinished prerequisite must remain pending",
        ));
    }
    if previous.succeeded() && *next == CreateStep::NotAttempted {
        return Err(invalid(
            "not_attempted must be justified by a failed prerequisite",
        ));
    }
    Ok(())
}
fn validate_delivery(
    message: Option<&CreateStep>,
    delivery: Option<Delivery>,
    queue: Option<InputQueue>,
    revision: Option<Revision>,
    load: bool,
    start_error: bool,
) -> Result<(), ValidationError> {
    let accepted = message.is_some_and(CreateStep::succeeded);
    if accepted {
        if delivery != Some(Delivery::Accepted) || queue.is_none() || revision.is_none() {
            return Err(invalid(
                "a succeeded message step requires all acceptance facts",
            ));
        }
    } else if delivery.is_some() || queue.is_some() || revision.is_some() || load || start_error {
        return Err(invalid(
            "an unaccepted or unrequested message cannot carry acceptance facts",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentCreateKind {
    #[serde(rename = "agent.create")]
    AgentCreate,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionCreateKind {
    #[serde(rename = "session.create")]
    SessionCreate,
}

crate::validated_dto!(AgentCreateReceipt, AgentCreateReceiptWire, "AgentCreateReceiptWire" {
    kind: AgentCreateKind,
    host_id: HostId,
    host_incarnation: HostIncarnation,
    operation_id: OperationId,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    message_id: Option<MessageId>,
    result: AgentCreateResult,
});
crate::validated_dto!(SessionCreateReceipt, SessionCreateReceiptWire, "SessionCreateReceiptWire" {
    kind: SessionCreateKind,
    host_id: HostId,
    host_incarnation: HostIncarnation,
    operation_id: OperationId,
    agent_id: AgentId,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    message_id: Option<MessageId>,
    result: SessionCreateResult,
});
impl AgentCreateReceipt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.result.validate()?;
        if self.agent_id.is_some() != self.result.steps.agent.succeeded()
            || self.session_id.is_some() != self.result.steps.session.succeeded()
            || self.message_id.is_some() != self.result.steps.message.is_some()
        {
            return Err(invalid(
                "creation identities must exactly reflect published objects and requested message",
            ));
        }
        Ok(())
    }
    /// Progress queries may include pending/running; the ordinary create call
    /// returns terminal stage facts, not a fabricated asynchronous success.
    pub fn validate_call_return(&self) -> Result<(), ValidationError> {
        self.validate()?;
        if self.result.steps.agent.in_progress()
            || self.result.steps.session.in_progress()
            || self
                .result
                .steps
                .message
                .as_ref()
                .is_some_and(CreateStep::in_progress)
        {
            Err(invalid(
                "ordinary creation return cannot contain pending/running steps",
            ))
        } else {
            Ok(())
        }
    }
}
impl SessionCreateReceipt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.result.validate()?;
        if self.session_id.is_some() != self.result.steps.session.succeeded()
            || self.message_id.is_some() != self.result.steps.message.is_some()
        {
            return Err(invalid(
                "creation identities must exactly reflect published objects and requested message",
            ));
        }
        Ok(())
    }
    pub fn validate_call_return(&self) -> Result<(), ValidationError> {
        self.validate()?;
        if self.result.steps.session.in_progress()
            || self
                .result
                .steps
                .message
                .as_ref()
                .is_some_and(CreateStep::in_progress)
        {
            Err(invalid(
                "ordinary creation return cannot contain pending/running steps",
            ))
        } else {
            Ok(())
        }
    }
}

/// Existing owner-held creation progress or formed control facts. Serialization
/// preserves the original method's receipt; this is not an Operation entity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Receipt {
    AgentCreate(AgentCreateReceipt),
    SessionCreate(SessionCreateReceipt),
    HostShutdown(crate::HostShutdownReceipt),
    HostReload(crate::HostReloadReceipt),
    AgentStop(crate::AgentStopReceipt),
    SessionControl(crate::SessionControlReceipt),
    SessionClose(crate::SessionCloseReceipt),
    TurnCancel(crate::TurnCancelReceipt),
    SessionCancel(crate::SessionCancelReceipt),
    InteractionAnswer(crate::InteractionAnswerReceipt),
}
impl Receipt {
    pub fn operation_id(&self) -> &OperationId {
        match self {
            Self::AgentCreate(receipt) => &receipt.operation_id,
            Self::SessionCreate(receipt) => &receipt.operation_id,
            Self::HostShutdown(receipt) => &receipt.operation_id,
            Self::HostReload(receipt) => &receipt.operation_id,
            Self::AgentStop(receipt) => &receipt.operation_id,
            Self::SessionControl(receipt) => &receipt.operation_id,
            Self::SessionClose(receipt) => &receipt.operation_id,
            Self::TurnCancel(receipt) => &receipt.operation_id,
            Self::SessionCancel(receipt) => &receipt.operation_id,
            Self::InteractionAnswer(receipt) => &receipt.operation_id,
        }
    }
}
