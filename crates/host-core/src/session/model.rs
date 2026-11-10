use super::*;
use futures_util::StreamExt;
use minicore_runtime::model::{
    Model, ModelCallContext, ModelDescriptor, ModelError, ModelErrorKind, ModelEvent, ModelRequest,
    ModelStartFuture, ModelStream,
};

/// Runtime checks identities within one response. The public Host identity is
/// (TurnId, original ToolCallId), including every response and internal Loop.
/// This binding lasts for that public Turn and reads its accepted records; it
/// does not reserve IDs from a partial response or a failed model attempt.
struct TurnModel {
    session: Weak<SessionController>,
    turn_id: TurnId,
    inner: Arc<dyn Model>,
    descriptor: ModelDescriptor,
}

impl Model for TurnModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        Box::pin(async move {
            let stream = self.inner.start(request, context).await?;
            let session = self.session.clone();
            let turn_id = self.turn_id.clone();
            Ok(Box::pin(stream.map(move |event| {
                if let Ok(ModelEvent::ToolCallStart { tool_call_id, .. }) = &event
                    && let Some(session) = session.upgrade()
                {
                    let state = session.state.lock().expect("Session mutex poisoned");
                    let duplicate = state.output.get(&turn_id).is_some_and(|turn| {
                        turn.tools
                            .contains_key(&ToolCallId(tool_call_id.as_str().to_owned()))
                    });
                    if duplicate {
                        // Reject before Runtime assembly: callback-only refusal
                        // leaves the invalid Assistant in the final report's
                        // unseen suffix, and may also expose it as a partial.
                        return Err(ModelError::started(
                            ModelErrorKind::InvalidProviderResponse,
                            DiagnosticSummary::new(
                                DiagnosticCode::ModelMalformedResponse,
                                DiagnosticCategory::Model,
                                Text::new(
                                    "model response repeats a ToolCallId already accepted by this Turn",
                                ),
                                false,
                            )
                            .expect("static model protocol diagnostic"),
                        ));
                    }
                }
                event
            })) as ModelStream)
        })
    }
}

impl SessionController {
    pub(super) fn bind_turn_model(
        self: &Arc<Self>,
        turn_id: &TurnId,
        execution: &ExecutionConfig,
    ) -> Result<ExecutionConfig, Fault> {
        ExecutionConfig::new(
            Arc::new(TurnModel {
                session: Arc::downgrade(self),
                turn_id: turn_id.clone(),
                inner: Arc::clone(execution.model()),
                descriptor: execution.descriptor().clone(),
            }),
            execution.reasoning(),
            execution.tools().clone(),
            execution.policy().cloned(),
            Arc::clone(execution.prompt()),
        )
        .map_err(|error| Fault::new(FaultKind::TurnInternalUnexpected, error.to_string()))
    }
}
