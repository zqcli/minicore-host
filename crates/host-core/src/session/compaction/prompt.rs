//! Session request preparation over a shared accepted history view.
use super::*;
use minicore_host_runtime::{
    HistoryView,
    model::{ModelLimits, ModelRequest},
    prompt::{PreparedPrompt, PromptError, PromptRequest},
};
use std::collections::BTreeSet;

/// Sharing the native buffer avoids copying full history bodies. Input metadata
/// is copied for this view; only actual compaction flattens the selected native
/// context, outside the owner lock.
pub(super) struct RequestHistory {
    raw: Arc<Vec<HistoryItem>>,
    summary: Option<HistoryItem>,
    input_provenance: input_provenance::InputProvenance,
    revision: u64,
    covered: usize,
    preserve_from: usize,
    pub(super) previous_coverage: u64,
}
impl RequestHistory {
    pub(super) fn new(state: &SessionState) -> Result<Self, Fault> {
        let covered = state
            .projection
            .as_ref()
            .map_or(0, |value| value.covered_items);
        let covered_index = usize::try_from(covered).map_err(|_| stale())?;
        if covered_index > state.history.len() {
            return Err(stale());
        }
        let summary = state
            .projection
            .as_ref()
            .map(|value| HistoryItem::Summary(value.summary.clone()));
        let prefix = usize::from(summary.is_some());
        let mut input_provenance = input_provenance::InputProvenance::default();
        input_provenance.append_range(
            &state.input_provenance,
            covered_index..state.history.len(),
            prefix,
        );
        let mut preserve = state
            .active
            .as_ref()
            .map_or(state.history.len(), |active| active.save_cursor);
        if let Some(start) = incomplete_tail(&state.history[..preserve]) {
            preserve = preserve.min(start);
        }
        Ok(Self {
            raw: Arc::clone(&state.history),
            summary,
            input_provenance,
            revision: state.history_position,
            covered: covered_index,
            preserve_from: prefix + preserve.saturating_sub(covered_index),
            previous_coverage: covered,
        })
    }
    pub(super) fn view(&self) -> HistoryView<'_> {
        HistoryView::new(self.summary.as_slice(), &self.raw[self.covered..])
    }
    pub(super) fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            revision: self.revision,
            items: self.view().iter().cloned().collect::<Vec<_>>().into(),
        }
    }
    pub(super) fn matches(&self, state: &SessionState) -> bool {
        self.revision == state.history_position
            && Arc::ptr_eq(&self.raw, &state.history)
            && self.previous_coverage == state.projection.as_ref().map_or(0, |p| p.covered_items)
    }
    async fn project(
        &self,
        execution: &ExecutionConfig,
        loop_id: LoopId,
        request_index: u32,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<PreparedPrompt, PromptError> {
        let tools = execution.tools().specs();
        let child = cancellation.child_token();
        let prepare = async {
            execution
                .prompt()
                .prepare(PromptRequest {
                    loop_id,
                    request_index,
                    history: self.view(),
                    model: execution.descriptor(),
                    reasoning: execution.reasoning(),
                    tools,
                    cancellation: child.clone(),
                    deadline,
                })
                .await
        };
        tokio::pin!(prepare);
        // Declared after the future so unwind/drop cancels the provider's
        // child before destroying its retained future.
        let _cancel = child.clone().drop_guard();
        let result = tokio::select! {
            _ = cancellation.cancelled() => Err(PromptError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(PromptError::Cancelled),
            result = &mut prepare => result,
        };
        child.cancel();
        self.input_provenance.project(self.view(), result?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn prepare_manual(
        &self,
        source: HistorySnapshot,
        resources: &crate::BoundCompactionResources,
        loop_id: LoopId,
        settings: &CompactionSettings,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<Option<engine::CompactionPlan>, Fault> {
        let child = cancellation.child_token();
        let prepare = async {
            resources
                .prompt
                .prepare(PromptRequest {
                    loop_id,
                    request_index: 0,
                    history: self.view(),
                    model: resources.model.descriptor(),
                    reasoning: resources.reasoning,
                    tools: &resources.tools,
                    cancellation: child.clone(),
                    deadline,
                })
                .await
        };
        tokio::pin!(prepare);
        let _cancel = child.clone().drop_guard();
        let prompt = tokio::select! {
            _ = cancellation.cancelled() => Err(PromptError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(PromptError::Cancelled),
            result = &mut prepare => result,
        };
        child.cancel();
        let prompt = prompt
            .and_then(|prompt| self.input_provenance.project(self.view(), prompt))
            .map_err(|error| failure(error.to_string()))?;
        let request = ModelRequest::new(
            prompt.messages,
            resources.tools.clone(),
            ModelLimits::default(),
            resources.reasoning,
        )
        .map_err(|error| failure(error.to_string()))?;
        engine::prepare(
            source,
            self.view().len(),
            resources.model.descriptor(),
            settings,
            request,
        )
        .map_err(|error| failure(error.to_string()))
    }
}

fn incomplete_tail(raw: &[HistoryItem]) -> Option<usize> {
    let start = raw
        .iter()
        .rposition(|item| !matches!(item, HistoryItem::ToolResult(_)))?;
    let HistoryItem::Assistant(assistant) = &raw[start] else {
        return None;
    };
    let results: BTreeSet<_> = raw[start + 1..]
        .iter()
        .filter_map(|item| match item {
            HistoryItem::ToolResult(result) => Some((
                result.loop_id,
                result.request_index,
                &result.call_id,
                &result.tool_name,
            )),
            _ => None,
        })
        .collect();
    assistant
        .content
        .iter()
        .filter_map(minicore_host_runtime::model::AssistantPart::as_tool_call)
        .any(|call| {
            !results.contains(&(
                assistant.loop_id,
                assistant.request_index,
                call.tool_call_id(),
                call.name(),
            ))
        })
        .then_some(start)
}

impl SessionController {
    pub(in crate::session) async fn prepare_request(
        self: &Arc<Self>,
        turn_id: &TurnId,
        request_index: u32,
        execution: &ExecutionConfig,
        cancellation: &CancellationToken,
        prompt_deadline: &mut tokio::time::Instant,
        options: &minicore_host_runtime::ExecutionOptions,
    ) -> Result<ModelRequest, Fault> {
        let prepare = self.prepare_request_inner(
            turn_id,
            request_index,
            execution,
            cancellation,
            prompt_deadline,
            options,
        );
        tokio::select! {
            _ = cancellation.cancelled() => Err(prompt_fault("prompt preparation cancelled")),
            result = AssertUnwindSafe(prepare).catch_unwind() => result.unwrap_or_else(|_| Err(prompt_fault("prompt preparation panicked"))),
        }
    }
    async fn prepare_request_inner(
        self: &Arc<Self>,
        turn_id: &TurnId,
        request_index: u32,
        execution: &ExecutionConfig,
        cancellation: &CancellationToken,
        prompt_deadline: &mut tokio::time::Instant,
        options: &minicore_host_runtime::ExecutionOptions,
    ) -> Result<ModelRequest, Fault> {
        let (history, loop_id, load_id, settings) = {
            let state = self.state.lock().expect("Session mutex poisoned");
            let active = state
                .active
                .as_ref()
                .filter(|active| &active.info.id == turn_id)
                .ok_or_else(stale)?;
            (
                RequestHistory::new(&state)?,
                active.loop_id,
                active.info.load_id.clone(),
                active.info.config.execution.compaction.clone(),
            )
        };
        let prompt = history
            .project(
                execution,
                loop_id,
                request_index,
                cancellation,
                *prompt_deadline,
            )
            .await
            .map_err(|error| prompt_fault(error.to_string()))?;
        let request = execution
            .request(prompt.messages)
            .map_err(|error| prompt_fault(error.to_string()))?;
        if !settings.enabled {
            return Ok(request);
        }
        let Some(plan) = engine::prepare_automatic(
            || history.snapshot(),
            history.preserve_from,
            execution.descriptor(),
            &settings,
            request.clone(),
        )
        .map_err(|error| failure(error.to_string()))?
        else {
            return Ok(request);
        };
        if plan.covered().end <= usize::from(history.previous_coverage > 0) {
            return Ok(request);
        }
        let source = plan.source().clone();
        let binding = CompactionBinding::Automatic {
            turn_id: turn_id.clone(),
            load_id,
            loop_id,
            request_index,
        };
        let compaction_deadline = options.deadline.map_or_else(
            || tokio::time::Instant::now() + options.model_timeout,
            |turn| turn.min(tokio::time::Instant::now() + options.model_timeout),
        );
        let mut completion = self.start_automatic(
            binding,
            source,
            history.previous_coverage,
            plan,
            execution.model().clone(),
            cancellation.child_token(),
            compaction_deadline,
        )?;
        // The registered compaction/save owns its work even when the caller's
        // timeout drops this wait. Finalization fences that original work.
        loop {
            if let Some(result) = completion.borrow_and_update().clone() {
                result?;
                break;
            }
            completion.changed().await.map_err(|_| stale())?;
        }
        drop(history);
        // Compaction is a separate operation. Its time does not consume or
        // reset an outstanding prompt call; this is a new projection of the
        // newly committed context. Steering rebuilds retain this deadline.
        *prompt_deadline = options.deadline.map_or_else(
            || tokio::time::Instant::now() + options.prompt_timeout,
            |turn| turn.min(tokio::time::Instant::now() + options.prompt_timeout),
        );
        let history = {
            let state = self.state.lock().expect("Session mutex poisoned");
            RequestHistory::new(&state)?
        };
        let prompt = history
            .project(
                execution,
                loop_id,
                request_index,
                cancellation,
                *prompt_deadline,
            )
            .await
            .map_err(|error| prompt_fault(error.to_string()))?;
        execution
            .request(prompt.messages)
            .map_err(|error| prompt_fault(error.to_string()))
    }
}

fn prompt_fault(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::TurnInternalUnexpected, message.into())
}

#[cfg(test)]
#[path = "../input_provenance_tests.rs"]
mod input_provenance_tests;

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
