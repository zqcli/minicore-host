//! Exact waiting points and answers owned by this Session.
use super::*;
use crate::{ActivityEvent, InteractionResolution, InteractionResolvedData, ObservationEvent};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAnswer {
    AllowOnce,
    Deny,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InteractionAnswerData {
    Approval {
        decision: ApprovalAnswer,
    },
    ToolInput {
        answer: minicore_host_runtime::tools::ToolInputAnswer,
    },
}
impl InteractionAnswerData {
    fn into_runtime(self) -> minicore_host_runtime::interaction::InteractionAnswer {
        use minicore_host_runtime::{interaction::InteractionAnswer, tools::ApprovalDecision};
        match self {
            Self::Approval { decision } => InteractionAnswer::Approval(match decision {
                ApprovalAnswer::AllowOnce => ApprovalDecision::AllowOnce,
                ApprovalAnswer::Deny => ApprovalDecision::Deny,
            }),
            Self::ToolInput { answer } => InteractionAnswer::ToolInput(answer),
        }
    }
}
pub(super) struct PendingAnswer {
    request: minicore_host_runtime::interaction::PendingInteraction,
    reply: tokio::sync::oneshot::Sender<minicore_host_runtime::interaction::InteractionAnswer>,
    pub(super) data: crate::PendingInteractionData,
}
pub(super) struct SessionToolHost {
    pub(super) session: Arc<SessionController>,
    pub(super) turn_id: TurnId,
}
impl minicore_host_runtime::tools::ToolBatchHost for SessionToolHost {
    fn observe(&self, event: minicore_host_runtime::tools::ToolBatchEvent) {
        self.session.observe_tool_event(&self.turn_id, event);
    }
    fn request_interaction(
        &self,
        pending: minicore_host_runtime::interaction::PendingInteraction,
        reply: tokio::sync::oneshot::Sender<minicore_host_runtime::interaction::InteractionAnswer>,
    ) -> Result<(), minicore_host_runtime::tools::ToolBatchStop> {
        use minicore_host_runtime::tools::ToolBatchStop;
        let mut state = self.session.state.lock().expect("Session mutex poisoned");
        if state.host_stopping || !self.session.access.execution_allowed_owned() {
            return Err(ToolBatchStop::Cancelled);
        }
        let active = state
            .active
            .as_ref()
            .filter(|a| {
                a.info.id == self.turn_id
                    && a.info.state == TurnState::Running
                    && !a.cancellation.is_cancelled()
            })
            .ok_or(ToolBatchStop::Cancelled)?;
        let source = self.session.turn_identity(&active.info);
        if active.pending_interaction.is_some() {
            return Err(ToolBatchStop::Internal(
                DiagnosticSummary::new(
                    DiagnosticCode::InteractionNotFound,
                    DiagnosticCategory::Internal,
                    Text::new("Turn already has a pending interaction"),
                    false,
                )
                .expect("static diagnostic"),
            ));
        }
        let record = state
            .output
            .entry(self.turn_id.clone())
            .or_default()
            .interaction(&pending);
        let data = record.data.clone();
        let active = state.active.as_mut().expect("checked Turn");
        active.pending_interaction = Some(PendingAnswer {
            request: pending,
            reply,
            data: data.clone(),
        });
        active.info.phase = Some(TurnPhase::WaitingForInput);
        self.session
            .access
            .observation
            .publish(ObservationEvent::Activity(
                ActivityEvent::InteractionRequested {
                    source: ToolCallIdentity {
                        turn: source,
                        tool_call_id: data.tool_call_id,
                    },
                    interaction_id: data.interaction_id,
                    data: data.request,
                },
            ));
        self.session.publish(&mut state);
        Ok(())
    }
    fn finish_interaction(&self, id: minicore_host_runtime::InteractionId) {
        let mut state = self.session.state.lock().expect("Session mutex poisoned");
        if state
            .active
            .as_ref()
            .is_none_or(|active| active.info.id != self.turn_id)
        {
            return;
        }
        let active = state.active.as_mut().expect("checked Turn");
        if active
            .pending_interaction
            .as_ref()
            .is_some_and(|pending| pending.request.interaction_id == id)
        {
            active.pending_interaction = None;
        }
        if active.info.state == TurnState::Running {
            active.info.phase = Some(TurnPhase::Tools);
        }
        self.session.publish(&mut state);
    }
}
impl SessionController {
    pub(crate) fn answer_interaction(
        &self,
        caller: &CallerContext,
        request: InteractionAnswerRequest<InteractionAnswerData>,
    ) -> Result<InteractionAnswerReceipt, Fault> {
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::available(&state)?;
            let active = state
                .active
                .as_ref()
                .filter(|active| active.info.id == request.turn_id)
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionTurnNotFound,
                        "the original Turn is not active",
                    )
                })?;
            if active.info.load_id != request.load_id {
                return Err(Fault::new(
                    FaultKind::SessionOwnershipMismatch,
                    "interaction belongs to another Load",
                ));
            }
            if active.info.state != TurnState::Running
                || active.cancellation.is_cancelled()
                || state.host_stopping
                || !self.access.execution_allowed_owned()
            {
                return Err(not_pending());
            }
            let source = self.turn_identity(&active.info);
            let pending = active
                .pending_interaction
                .as_ref()
                .filter(|pending| pending.data.interaction_id == request.interaction_id)
                .ok_or_else(not_pending)?;
            let answer = request.answer.into_runtime();
            pending.request.validate_answer(&answer).map_err(|_| {
                Fault::new(
                    FaultKind::SessionInteractionAnswerInvalid,
                    "answer does not match the original pending request",
                )
            })?;
            // Validation, consumption and answer-vs-cancel share one short
            // owner boundary. A oneshot send never awaits the tool primitive.
            let pending = state
                .active
                .as_mut()
                .expect("checked Turn")
                .pending_interaction
                .take()
                .expect("checked slot");
            pending.reply.send(answer).map_err(|_| not_pending())?;
            let record = state
                .output
                .get_mut(&request.turn_id)
                .and_then(|output| output.interactions.get_mut(&pending.request.interaction_id))
                .expect("display record created with pending slot");
            record.resolved = true;
            self.access.observation.publish(ObservationEvent::Activity(
                ActivityEvent::InteractionResolved {
                    source: ToolCallIdentity {
                        turn: source.clone(),
                        tool_call_id: record.data.tool_call_id.clone(),
                    },
                    interaction_id: request.interaction_id.clone(),
                    data: InteractionResolvedData {
                        resolution: InteractionResolution::Answered,
                    },
                },
            ));
            let receipt = InteractionAnswerReceipt {
                kind: InteractionAnswerKind::InteractionAnswer,
                host_id: source.session.host_id,
                host_incarnation: source.session.host_incarnation,
                agent_id: source.session.agent_id,
                session_id: source.session.session_id,
                turn_id: request.turn_id,
                load_id: request.load_id,
                interaction_id: request.interaction_id,
                operation_id: request.operation_id,
                result: InteractionAnswerResult {
                    status: InteractionAnswerStatus::Answered,
                },
            };
            Self::retain_control(&mut state, Receipt::InteractionAnswer(receipt.clone()));
            self.publish(&mut state);
            Ok(receipt)
        })
    }
}
fn not_pending() -> Fault {
    Fault::new(
        FaultKind::SessionInteractionNotPending,
        "the original interaction is no longer pending",
    )
}
