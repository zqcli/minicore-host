//! Runtime prompt preparation over the Session's accepted history and projection.
use super::*;
use minicore_runtime::{
    HistoryView,
    model::{ModelLimits, ModelRequest},
    prompt::{PreparedPrompt, PromptError, PromptFuture, PromptProvider, PromptRequest},
};

/// Base/appended retain the actual internal Loop distinction. Only the
/// unhanded suffix is protected; earlier batches of this same Turn are eligible.
pub(super) struct RequestHistory {
    input_provenance: input_provenance::InputProvenance,
    pub(super) source: HistorySnapshot,
    base_len: usize,
    preserve_from: usize,
    pub(super) previous_coverage: u64,
}
impl RequestHistory {
    pub(super) fn new(
        state: &SessionState,
        raw_base: usize,
        unhanded: &[HistoryItem],
    ) -> Result<Self, Fault> {
        let covered = state
            .projection
            .as_ref()
            .map_or(0, |value| value.covered_items);
        let covered_index = usize::try_from(covered).map_err(|_| stale())?;
        if raw_base > state.history.len() || covered_index > state.history.len() {
            return Err(stale());
        }
        let mut items = state
            .projection
            .as_ref()
            .map(|value| vec![HistoryItem::Summary(value.summary.clone())])
            .unwrap_or_default();
        let mut input_provenance = input_provenance::InputProvenance::default();
        if covered_index < raw_base {
            input_provenance.append_range(
                &state.input_provenance,
                covered_index..raw_base,
                items.len(),
            );
            items.extend_from_slice(&state.history[covered_index..raw_base]);
        }
        let base_len = items.len();
        input_provenance.append_range(
            &state.input_provenance,
            raw_base.max(covered_index)..state.history.len(),
            items.len(),
        );
        items.extend_from_slice(&state.history[raw_base.max(covered_index)..]);
        let mut preserve_from = items.len();
        // A genuinely incomplete *accepted* tail is not a closed exchange for
        // the durable marker. Fresh, still-unhanded input cannot close it in
        // the journal. Keep that one group intact until a later accepted item
        // establishes its end, just as the storage coverage validator requires.
        if let Some(start) = incomplete_tail(&state.history) {
            preserve_from = usize::from(state.projection.is_some()) + start - covered_index;
        }
        input_provenance.append_unhanded(items.len(), unhanded, state.active.as_ref())?;
        items.extend_from_slice(unhanded);
        Ok(Self {
            input_provenance,
            source: HistorySnapshot {
                revision: state.history_position,
                items: items.into(),
            },
            base_len,
            preserve_from,
            previous_coverage: covered,
        })
    }
    fn view(&self) -> HistoryView<'_> {
        HistoryView::new(
            &self.source.items[..self.base_len],
            &self.source.items[self.base_len..],
        )
    }
}

impl RequestHistory {
    pub(super) async fn prepare_manual(
        &self,
        resources: &crate::BoundCompactionResources,
        loop_id: LoopId,
        settings: &CompactionSettings,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<Option<engine::CompactionPlan>, Fault> {
        let source = &self.source;
        let prompt = resources.prompt.prepare(PromptRequest {
            loop_id,
            request_index: 0,
            history: HistoryView::new(&source.items, &[]),
            model: resources.model.descriptor(),
            reasoning: resources.reasoning,
            tools: &resources.tools,
            cancellation: cancellation.clone(),
            deadline,
        });
        let prompt = tokio::select! {
            _ = cancellation.cancelled() => Err(PromptError::Cancelled),
            result = tokio::time::timeout_at(deadline, prompt) => result.unwrap_or(Err(PromptError::Cancelled)),
        };
        prompt
            .and_then(|prompt| self.input_provenance.project(&source.items, prompt))
            .map_err(|error| failure(error.to_string()))
            .and_then(|prompt| {
                let request = ModelRequest::new(
                    prompt.messages,
                    resources.tools.clone(),
                    ModelLimits::default(),
                    resources.reasoning,
                )
                .map_err(|error| failure(error.to_string()))?;
                engine::prepare(
                    source.clone(),
                    source.items.len(),
                    resources.model.descriptor(),
                    settings,
                    request,
                )
                .map_err(|error| failure(error.to_string()))
            })
    }
}

fn incomplete_tail(raw: &[HistoryItem]) -> Option<usize> {
    let start = raw
        .iter()
        .rposition(|item| !matches!(item, HistoryItem::ToolResult(_)))?;
    let HistoryItem::Assistant(assistant) = &raw[start] else {
        return None;
    };
    assistant
        .content
        .iter()
        .filter_map(minicore_runtime::model::AssistantPart::as_tool_call)
        .any(|call| {
            !raw[start + 1..].iter().any(|item| matches!(item,
            HistoryItem::ToolResult(result) if result.loop_id == assistant.loop_id
                && result.request_index == assistant.request_index
                && &result.call_id == call.tool_call_id() && &result.tool_name == call.name()
        ))
        })
        .then_some(start)
}

impl SessionController {
    pub(in crate::session) fn bind_compaction_prompt(
        self: &Arc<Self>,
        turn_id: &TurnId,
        config: &SessionConfig,
        execution: &ExecutionConfig,
    ) -> Result<ExecutionConfig, Fault> {
        ExecutionConfig::new(
            execution.model().clone(),
            execution.reasoning(),
            execution.tools().clone(),
            execution.policy().cloned(),
            Arc::new(SessionPrompt {
                session: Arc::downgrade(self),
                turn_id: turn_id.clone(),
                inner: execution.prompt().clone(),
                model: execution.model().clone(),
                settings: config.execution.compaction.clone(),
            }),
        )
        .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))
    }
    fn request_history(
        &self,
        turn_id: &TurnId,
        request: &PromptRequest<'_>,
    ) -> Result<(RequestHistory, LoadId), Fault> {
        let state = self.state.lock().expect("Session mutex poisoned");
        let active = state
            .active
            .as_ref()
            .filter(|active| &active.info.id == turn_id && active.loop_id == Some(request.loop_id))
            .ok_or_else(stale)?;
        if active.accepted > request.history.appended().len()
            || state.history.len() != active.raw_base_len + active.accepted
            || request.history.base().len() != active.raw_base_len
        {
            return Err(stale());
        }
        Ok((
            RequestHistory::new(
                &state,
                active.raw_base_len,
                &request.history.appended()[active.accepted..],
            )?,
            active.info.load_id.clone(),
        ))
    }
}
struct SessionPrompt {
    session: Weak<SessionController>,
    turn_id: TurnId,
    inner: Arc<dyn PromptProvider>,
    model: Arc<dyn Model>,
    settings: CompactionSettings,
}
impl PromptProvider for SessionPrompt {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        Box::pin(async move {
            let owner = self.session.upgrade().ok_or(PromptError::Cancelled)?;
            let (history, load_id) = owner
                .request_history(&self.turn_id, &request)
                .map_err(|_| PromptError::InvalidHistory)?;
            let prompt = self.project(&history, &request).await?;
            if !self.settings.enabled {
                return Ok(prompt);
            }
            let normal = ModelRequest::new(
                prompt.messages.clone(),
                request.tools.to_vec(),
                ModelLimits::default(),
                request.reasoning,
            )
            .map_err(|_| PromptError::InvalidHistory)?;
            if !engine::estimate_context(request.model, &self.settings, &normal)
                .map_err(|_| PromptError::InvalidHistory)?
                .automatic_due
            {
                return Ok(prompt);
            }
            let Some(plan) = engine::prepare(
                history.source.clone(),
                history.preserve_from,
                request.model,
                &self.settings,
                normal,
            )
            .map_err(|_| PromptError::InvalidHistory)?
            else {
                return Ok(prompt);
            };
            // A/F may rebuild the same request after input arrives. Merely
            // resummarizing the previous summary is not progress or new work.
            if plan.covered().end <= usize::from(history.previous_coverage > 0) {
                return Ok(prompt);
            }
            let binding = CompactionBinding::Automatic {
                turn_id: self.turn_id.clone(),
                load_id,
                loop_id: request.loop_id,
                request_index: request.request_index,
            };
            let mut completion = owner
                .start_automatic(
                    binding,
                    &history,
                    plan,
                    self.model.clone(),
                    request.cancellation.child_token(),
                    request.deadline,
                )
                .map_err(|_| PromptError::Cancelled)?;
            loop {
                if let Some(result) = completion.borrow_and_update().clone() {
                    result.map_err(|_| PromptError::InvalidHistory)?;
                    break;
                }
                completion
                    .changed()
                    .await
                    .map_err(|_| PromptError::Cancelled)?;
            }
            let (history, _) = owner
                .request_history(&self.turn_id, &request)
                .map_err(|_| PromptError::InvalidHistory)?;
            self.project(&history, &request).await
        })
    }
}
impl SessionPrompt {
    async fn project(
        &self,
        history: &RequestHistory,
        request: &PromptRequest<'_>,
    ) -> Result<PreparedPrompt, PromptError> {
        let prompt = self
            .inner
            .prepare(PromptRequest {
                loop_id: request.loop_id,
                request_index: request.request_index,
                history: history.view(),
                model: request.model,
                reasoning: request.reasoning,
                tools: request.tools,
                cancellation: request.cancellation.clone(),
                deadline: request.deadline,
            })
            .await?;
        history
            .input_provenance
            .project(&history.source.items, prompt)
    }
}

#[cfg(test)]
#[path = "../input_provenance_tests.rs"]
mod input_provenance_tests;
