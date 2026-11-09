use super::*;
use crate::context::*;
use minicore_host_storage::CompactionProjection;

impl SessionController {
    /// A pure owner snapshot: no file/resource reads, prompt preparation,
    /// Runner loading, input consumption, or changes to save/execution state.
    pub(crate) async fn context(
        &self,
        caller: &CallerContext,
    ) -> Result<SessionContextSnapshot, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            visible_context(&state)?;
            Ok(self.context_snapshot(&state))
        })
    }

    fn context_snapshot(&self, state: &SessionState) -> SessionContextSnapshot {
        SessionContextSnapshot {
            source: self.identity(),
            data: SessionContextData {
                controller_revision: state.controller_revision,
                residency: state.residency,
                load_id: state.load_id.clone(),
                history_loaded: state.history_loaded,
                config_revision: state.info.config_revision,
                model: state.info.config.model.clone(),
                compaction_settings: state.info.config.execution.compaction.clone(),
                active_turn: state.active.as_ref().map(|active| ContextTurnBinding {
                    turn_id: active.info.id.clone(),
                    load_id: active.info.load_id.clone(),
                    config_revision: active.info.config_revision,
                    model: active.context_binding.model.clone(),
                    compaction_settings: active.context_binding.compaction_settings.clone(),
                }),
                queued_inputs: ContextQueuedInputs {
                    next_step: state.next_step.len() as u64,
                    next_turn: state.next_turn.len() as u64,
                },
                history: state.readable_history.as_ref().map(|_| {
                    context_history(
                        state.history_position,
                        &state.history,
                        state.projection.as_ref(),
                    )
                }),
                compaction: state
                    .compaction
                    .as_ref()
                    .map(|work| work.activity.clone())
                    .or_else(|| state.last_compaction.clone()),
                history_save: ContextHistorySave {
                    pending: state.history_work.is_some(),
                    running: state.saving,
                    fault: state
                        .history_work
                        .as_ref()
                        .and_then(|work| work.fault.clone()),
                },
                block_reason: state.block_reason.clone(),
            },
        }
    }
}

fn visible_context(state: &SessionState) -> Result<(), Fault> {
    if state.reclaimed {
        Err(Fault::new(
            FaultKind::AgentSessionNotFound,
            "memory Session has been reclaimed",
        ))
    } else {
        Ok(())
    }
}

fn context_history(
    position: u64,
    raw: &[HistoryItem],
    projection: Option<&CompactionProjection>,
) -> ContextHistory {
    // These records have passed the existing owner/storage validation.
    let covered = projection.map_or(0, |projection| projection.covered_items);
    ContextHistory {
        position,
        raw_items: raw.len() as u64,
        effective_items: raw.len() as u64 - covered + u64::from(projection.is_some()),
        projection: projection.map(|projection| ContextProjection {
            source_position: projection.source_position,
            covered_items: projection.covered_items,
            summary: projection.summary.clone(),
        }),
        latest_response: raw.iter().enumerate().rev().find_map(|(index, item)| {
            let HistoryItem::Assistant(response) = item else {
                return None;
            };
            Some(ContextResponseUsage {
                raw_item_index: index as u64,
                model: response.model.clone(),
                covered_by_compaction: (index as u64) < covered,
                usage: response.usage,
            })
        }),
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
