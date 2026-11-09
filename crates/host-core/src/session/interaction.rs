//! Exact waiting-point answers. A private fence protects only the short gap
//! between owner acceptance and the Runtime control result; no owner lock is
//! held across the Runtime call or a wait.
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
        answer: minicore_runtime::tools::ToolInputAnswer,
    },
}
impl InteractionAnswerData {
    fn into_runtime(self) -> minicore_runtime::interaction::InteractionAnswer {
        use minicore_runtime::{interaction::InteractionAnswer, tools::ApprovalDecision};
        match self {
            Self::Approval { decision } => InteractionAnswer::Approval(match decision {
                ApprovalAnswer::AllowOnce => ApprovalDecision::AllowOnce,
                ApprovalAnswer::Deny => ApprovalDecision::Deny,
            }),
            Self::ToolInput { answer } => InteractionAnswer::ToolInput(answer),
        }
    }
}
impl SessionController {
    pub(crate) fn answer_interaction(
        &self,
        caller: &CallerContext,
        request: InteractionAnswerRequest<InteractionAnswerData>,
    ) -> Result<InteractionAnswerReceipt, Fault> {
        let (handle, runtime_id, source, completed) = self.access.accept(caller, || {
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
            if active.info.state != TurnState::Running {
                return Err(not_pending());
            }
            let handle = active.handle.clone().ok_or_else(not_pending)?;
            let source = TurnIdentity {
                session: self.identity(),
                turn_id: active.info.id.clone(),
                load_id: Some(active.info.load_id.clone()),
            };
            let (runtime_id, record) = state
                .output
                .get_mut(&request.turn_id)
                .and_then(|output| {
                    output
                        .interactions
                        .iter_mut()
                        .find(|(_, record)| record.data.interaction_id == request.interaction_id)
                })
                .ok_or_else(not_pending)?;
            if record.resolved || record.answering.is_some() {
                return Err(not_pending());
            }
            let (completed, fence) = watch::channel(false);
            record.answering = Some(fence);
            Ok((handle, *runtime_id, source, completed))
        })?;
        // This exact Runtime slot decides answer-vs-cancel. A later connection
        // revocation does not undo the owner-accepted attempt or rebind it.
        let result = handle
            .answer(runtime_id, request.answer.into_runtime())
            .map_err(|error| {
                use minicore_runtime::AnswerError;
                match error {
                    AnswerError::InteractionNotFound | AnswerError::NotActive => not_pending(),
                    AnswerError::WrongInteraction => Fault::new(
                        FaultKind::SessionInteractionAnswerInvalid,
                        "answer does not match the original pending request",
                    ),
                    _ => Fault::new(
                        FaultKind::SessionInternalUnexpected,
                        "Runtime returned an unrecognized interaction error",
                    ),
                }
            });
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let record = state
            .output
            .get_mut(&request.turn_id)
            .and_then(|output| output.interactions.get_mut(&runtime_id))
            .expect("original interaction retained until answer fence completes");
        record.answering = None;
        if result.is_ok() && !record.resolved {
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
        }
        let result = result.map(|()| {
            if let Some(active) = state.active.as_mut()
                && active.info.id == request.turn_id
                && active
                    .pending_interaction
                    .as_ref()
                    .is_some_and(|pending| pending.interaction_id == request.interaction_id)
            {
                active.pending_interaction = None;
            }
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
            receipt
        });
        completed.send_replace(true);
        result
    }
    /// Finalizing is already claimed before this call, so no new answers can
    /// enter. Only these exact in-flight decisions must settle before reclamation.
    pub(super) async fn finish_pending_answers(&self, turn: &TurnId) {
        let pending = {
            let state = self.state.lock().expect("Session mutex poisoned");
            state
                .output
                .get(turn)
                .into_iter()
                .flat_map(|output| output.interactions.values())
                .filter_map(|record| record.answering.clone())
                .collect::<Vec<_>>()
        };
        for mut completed in pending {
            let _ = completed.wait_for(|done| *done).await;
        }
    }
}
fn not_pending() -> Fault {
    Fault::new(
        FaultKind::SessionInteractionNotPending,
        "the original interaction is no longer pending",
    )
}
