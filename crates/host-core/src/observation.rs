//! Connection-bound dispatch. Source owners retain bodies; this facility never does.
use crate::control::{CallerContext, ConnectionRegistry};
use crate::types::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{AssistantHistory, ToolResultHistory};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostStateData {
    pub lifecycle: HostLifecycle,
    pub shutdown_strategy: Option<HostShutdownStrategy>,
    pub shutdown_fault: Option<Fault>,
    pub control: ControlStateData,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnStateData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_position: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<Timestamp>,
    pub state: TurnState,
    pub runner_revision: u64,
    pub config_revision: ConfigRevision,
    pub outcome: Option<TurnOutcome>,
    pub persistence: PersistenceState,
    pub requests: u64,
    pub tool_rounds: u64,
    pub phase: Option<TurnPhase>,
    pub model_retry: Option<ModelRetryProgress>,
    pub model_attempts: Vec<u8>,
    pub fault: Option<Fault>,
    pub persistence_fault: Option<Fault>,
    #[serde(default)]
    pub cleanup_faults: Vec<Fault>,
}
/// The same atomic Turn identity and state used by subscription baselines.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnSnapshot {
    #[serde(flatten)]
    pub source: TurnIdentity,
    pub data: TurnStateData,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallState {
    Pending,
    Running,
    Finished,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolProgressData {
    pub message: Option<String>,
    pub completed: Option<u64>,
    pub total: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallStateData {
    pub runner_revision: u64,
    pub response_index: u64,
    pub name: String,
    pub state: ToolCallState,
    pub progress: Option<ToolProgressData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegations: Vec<crate::DelegationAssociation>,
    pub outcome: Option<minicore_host_runtime::tools::ToolResultOutcome>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallData {
    #[serde(flatten)]
    pub state: ToolCallStateData,
    pub arguments: serde_json::Value,
    pub result: Option<ToolResultData>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelResponseData {
    pub content: Vec<minicore_host_runtime::model::AssistantPart>,
    pub finish_reason: minicore_host_runtime::model::ModelFinishReason,
    pub usage: minicore_host_runtime::model::Usage,
}
impl From<&AssistantHistory> for ModelResponseData {
    fn from(value: &AssistantHistory) -> Self {
        Self {
            content: value.content.clone(),
            finish_reason: value.finish_reason,
            usage: value.usage,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResultData {
    pub outcome: minicore_host_runtime::tools::ToolResultOutcome,
    pub output: minicore_host_runtime::tools::ToolOutput,
}
impl From<&ToolResultHistory> for ToolResultData {
    fn from(value: &ToolResultHistory) -> Self {
        Self {
            outcome: value.outcome,
            output: value.output.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolInvocationData {
    pub name: String,
    pub arguments: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelFinishedData {
    pub history_position: u64,
    pub recorded_at: Timestamp,
    pub result: ModelResponseData,
    pub output_positions: Vec<ChannelPosition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolFinishedData {
    pub history_position: u64,
    pub recorded_at: Timestamp,
    pub result: ToolResultData,
    pub output_positions: Vec<ChannelPosition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StateEvent {
    #[serde(rename = "host.state")]
    Host {
        host_id: HostId,
        host_incarnation: HostIncarnation,
        data: HostStateData,
    },
    #[serde(rename = "session.state")]
    Session {
        #[serde(flatten)]
        source: SessionIdentity,
        data: SessionStateData,
    },
    #[serde(rename = "turn.state")]
    Turn {
        #[serde(flatten)]
        source: TurnIdentity,
        data: TurnStateData,
    },
    #[serde(rename = "toolcall.state")]
    Toolcall {
        #[serde(flatten)]
        source: ToolCallIdentity,
        data: ToolCallStateData,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObservationEvent {
    State(StateEvent),
    Activity(ActivityEvent),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRiskData {
    Low,
    Medium,
    High,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionRequestData {
    Approval {
        prompt: String,
        risk: ApprovalRiskData,
    },
    ToolInput {
        request: minicore_host_runtime::tools::ToolInputRequest,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingInteractionData {
    pub interaction_id: InteractionId,
    pub tool_call_id: ToolCallId,
    pub name: String,
    pub request: InteractionRequestData,
}
impl PendingInteractionData {
    pub(crate) fn from_runtime(
        value: &minicore_host_runtime::interaction::PendingInteraction,
        interaction_id: InteractionId,
    ) -> Self {
        let request = match &value.kind {
            minicore_host_runtime::interaction::InteractionKind::ToolInput(request) => {
                InteractionRequestData::ToolInput {
                    request: request.clone(),
                }
            }
            minicore_host_runtime::interaction::InteractionKind::Approval(request) => {
                InteractionRequestData::Approval {
                    prompt: request.prompt.as_str().to_owned(),
                    risk: match request.risk {
                        minicore_host_runtime::tools::ApprovalRisk::Low => ApprovalRiskData::Low,
                        minicore_host_runtime::tools::ApprovalRisk::Medium => {
                            ApprovalRiskData::Medium
                        }
                        minicore_host_runtime::tools::ApprovalRisk::High => ApprovalRiskData::High,
                    },
                }
            }
        };
        Self {
            interaction_id,
            tool_call_id: ToolCallId(value.tool_call_id.as_str().to_owned()),
            name: value.tool_name.as_str().to_owned(),
            request,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InteractionResolvedData {
    pub resolution: InteractionResolution,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionResolution {
    Answered,
    Cancelled,
    Expired,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageEventData {
    pub received_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_position: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<Timestamp>,
    pub text: String,
    pub mode: MessageMode,
    pub source: MessageSource,
    pub queue: InputQueue,
    pub order: u64,
    pub in_reply_to: Option<MessageId>,
    pub controller_revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ActivityEvent {
    #[serde(rename = "interaction.requested")]
    InteractionRequested {
        #[serde(flatten)]
        source: ToolCallIdentity,
        interaction_id: InteractionId,
        data: InteractionRequestData,
    },
    #[serde(rename = "interaction.resolved")]
    InteractionResolved {
        #[serde(flatten)]
        source: ToolCallIdentity,
        interaction_id: InteractionId,
        data: InteractionResolvedData,
    },
    #[serde(rename = "toolcall.started")]
    ToolStarted {
        #[serde(flatten)]
        source: ToolCallIdentity,
        data: ToolInvocationData,
    },
    #[serde(rename = "model.output.delta")]
    ModelOutput {
        #[serde(flatten)]
        source: ModelOutputIdentity,
        data: OutputDelta,
    },
    #[serde(rename = "toolcall.output.delta")]
    ToolOutput {
        #[serde(flatten)]
        source: ToolCallIdentity,
        data: OutputDelta,
    },
    #[serde(rename = "model.response.finished")]
    ModelFinished {
        #[serde(flatten)]
        source: ModelOutputIdentity,
        data: ModelFinishedData,
    },
    #[serde(rename = "toolcall.finished")]
    ToolFinished {
        #[serde(flatten)]
        source: ToolCallIdentity,
        data: ToolFinishedData,
    },
    #[serde(rename = "turn.finished")]
    TurnFinished {
        #[serde(flatten)]
        source: TurnIdentity,
        data: TurnStateData,
    },
    #[serde(rename = "session.closed")]
    SessionClosed {
        #[serde(flatten)]
        source: SessionIdentity,
        data: SessionStateData,
    },
    #[serde(rename = "message.accepted")]
    MessageAccepted {
        #[serde(flatten)]
        source: SessionIdentity,
        message_id: MessageId,
        data: MessageEventData,
    },
    #[serde(rename = "message.bound")]
    MessageBound {
        #[serde(flatten)]
        source: TurnIdentity,
        message_id: MessageId,
        data: MessageEventData,
    },
}
impl ObservationEvent {
    pub fn category(&self) -> EventCategory {
        match self {
            Self::State(_) => EventCategory::State,
            Self::Activity(
                ActivityEvent::ModelOutput { .. }
                | ActivityEvent::ToolOutput { .. }
                | ActivityEvent::ToolStarted { .. },
            ) => EventCategory::Output,
            Self::Activity(
                ActivityEvent::MessageAccepted { .. } | ActivityEvent::MessageBound { .. },
            ) => EventCategory::Message,
            Self::Activity(
                ActivityEvent::InteractionRequested { .. }
                | ActivityEvent::InteractionResolved { .. },
            ) => EventCategory::Interaction,
            Self::Activity(_) => EventCategory::Completion,
        }
    }
    fn identity(
        &self,
    ) -> (
        &HostId,
        Option<&AgentId>,
        Option<&SessionId>,
        Option<&TurnId>,
    ) {
        let (session, turn) = match self {
            Self::State(StateEvent::Host { host_id, .. }) => return (host_id, None, None, None),
            Self::State(StateEvent::Session { source, .. }) => (source, None),
            Self::State(StateEvent::Turn { source, .. }) => {
                (&source.session, Some(&source.turn_id))
            }
            Self::State(StateEvent::Toolcall { source, .. }) => {
                (&source.turn.session, Some(&source.turn.turn_id))
            }
            Self::Activity(
                ActivityEvent::ModelOutput { source, .. }
                | ActivityEvent::ModelFinished { source, .. },
            ) => (&source.turn.session, Some(&source.turn.turn_id)),
            Self::Activity(
                ActivityEvent::ToolOutput { source, .. }
                | ActivityEvent::ToolFinished { source, .. }
                | ActivityEvent::ToolStarted { source, .. }
                | ActivityEvent::InteractionRequested { source, .. }
                | ActivityEvent::InteractionResolved { source, .. },
            ) => (&source.turn.session, Some(&source.turn.turn_id)),
            Self::Activity(
                ActivityEvent::TurnFinished { source, .. }
                | ActivityEvent::MessageBound { source, .. },
            ) => (&source.session, Some(&source.turn_id)),
            Self::Activity(
                ActivityEvent::SessionClosed { source, .. }
                | ActivityEvent::MessageAccepted { source, .. },
            ) => (source, None),
        };
        (
            &session.host_id,
            Some(&session.agent_id),
            Some(&session.session_id),
            turn,
        )
    }
    fn matches(&self, scope: &SubscriptionScope) -> bool {
        let (host, agent, session, turn) = self.identity();
        match scope {
            SubscriptionScope::Host { host_id } => host == host_id,
            SubscriptionScope::Agent { host_id, agent_id } => {
                host == host_id && agent == Some(agent_id)
            }
            SubscriptionScope::Session {
                agent_id,
                session_id,
            } => agent == Some(agent_id) && session == Some(session_id),
            SubscriptionScope::Turn {
                session_id,
                turn_id,
            } => session == Some(session_id) && turn == Some(turn_id),
        }
    }
}
/// Control notifications deliberately sit outside the business-event envelope.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "notification", content = "data", rename_all = "snake_case")]
pub enum ObservationNotification {
    Event(Box<SubscriptionPush<ObservationEvent>>),
    ResyncRequired(ResyncRequired),
}
struct Registration {
    connection: u64,
    scope: SubscriptionScope,
    categories: BTreeSet<EventCategory>,
    sender: mpsc::UnboundedSender<ObservationNotification>,
}
pub(crate) struct ObservationHub {
    subscriptions: Mutex<BTreeMap<SubscriptionId, Registration>>,
}
impl ObservationHub {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            subscriptions: Mutex::new(BTreeMap::new()),
        })
    }
    pub(crate) fn register(
        self: &Arc<Self>,
        caller: &CallerContext,
        access: &Arc<ConnectionRegistry>,
        request: SubscribeRequest,
    ) -> Result<EventSubscription, Fault> {
        request
            .validate()
            .map_err(|e| Fault::new(FaultKind::HostObservationInvalid, e.to_string()))?;
        let id = SubscriptionId::new()
            .map_err(|e| Fault::new(FaultKind::HostInternalUnexpected, e.to_string()))?;
        let (sender, receiver) = mpsc::unbounded_channel();
        self.subscriptions
            .lock()
            .expect("observation mutex poisoned")
            .insert(
                id.clone(),
                Registration {
                    connection: caller.connection_number(),
                    scope: request.scope,
                    categories: request.categories,
                    sender,
                },
            );
        Ok(EventSubscription {
            hub: Arc::downgrade(self),
            caller: caller.clone(),
            access: Arc::clone(access),
            receiver,
            result: SubscribeResult {
                subscription_id: id,
                initial_states: Vec::new(),
                output_positions: Vec::new(),
            },
        })
    }
    pub(crate) fn publish(&self, event: ObservationEvent) {
        self.subscriptions
            .lock()
            .expect("observation mutex poisoned")
            .retain(|id, entry| {
                !entry.categories.contains(&event.category())
                    || !event.matches(&entry.scope)
                    || entry
                        .sender
                        .send(ObservationNotification::Event(Box::new(SubscriptionPush {
                            subscription_id: id.clone(),
                            event: event.clone(),
                        })))
                        .is_ok()
            });
    }
    pub(crate) fn close_connection(&self, connection: u64) {
        self.subscriptions
            .lock()
            .expect("observation mutex poisoned")
            .retain(|_, entry| entry.connection != connection);
    }
    pub(crate) fn unsubscribe(
        &self,
        connection: u64,
        id: &SubscriptionId,
    ) -> Result<UnsubscribeResult, Fault> {
        let mut entries = self
            .subscriptions
            .lock()
            .expect("observation mutex poisoned");
        if let Some(entry) = entries.get(id)
            && entry.connection != connection
        {
            return Err(Fault::new(
                FaultKind::HostAccessInvalid,
                "subscription belongs to another connection",
            ));
        }
        Ok(UnsubscribeResult {
            ended: entries.remove(id).is_some(),
        })
    }
    /// Called only when dispatch itself knows that this baseline is invalid.
    /// It is not a client RPC and does not infer loss from ordinary slow consumers.
    pub(crate) fn resync(&self, id: &SubscriptionId, fault: Fault) {
        if let Some(entry) = self
            .subscriptions
            .lock()
            .expect("observation mutex poisoned")
            .remove(id)
        {
            let _ = entry
                .sender
                .send(ObservationNotification::ResyncRequired(ResyncRequired {
                    subscription_id: id.clone(),
                    fault,
                }));
        }
    }
}
pub struct EventSubscription {
    hub: Weak<ObservationHub>,
    caller: CallerContext,
    access: Arc<ConnectionRegistry>,
    receiver: mpsc::UnboundedReceiver<ObservationNotification>,
    pub(crate) result: SubscribeResult<StateEvent>,
}
impl EventSubscription {
    pub fn result(&self) -> &SubscribeResult<StateEvent> {
        &self.result
    }
    /// Transport adapters call this only after a known dispatch loss. This is
    /// subscription control, not a remotely callable business method.
    pub fn end_after_delivery_failure(&self, fault: Fault) {
        if let Some(hub) = self.hub.upgrade() {
            hub.resync(&self.result.subscription_id, fault);
        }
    }
    pub async fn recv(&mut self) -> Result<Option<ObservationNotification>, Fault> {
        self.access.read(&self.caller, || Ok(()))?;
        let mut revoked = self.caller.revocation();
        tokio::select! {
            biased;
            _ = revoked.changed() => Err(Fault::new(FaultKind::HostAccessInvalid, "observation connection ended")),
            value = self.receiver.recv() => self.access.read(&self.caller, || Ok(value)),
        }
    }
}
impl Drop for EventSubscription {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            let _ = hub.unsubscribe(
                self.caller.connection_number(),
                &self.result.subscription_id,
            );
        }
    }
}
