use super::*;
use crate::{OwnedCompactionWork, OwnedToolWork, SessionOwnedWork, SessionSummary, ToolCallState};

impl SessionController {
    pub(super) fn owned_work(state: &SessionState) -> Box<SessionOwnedWork> {
        let active = state.active.as_ref();
        let tools = active
            .and_then(|active| state.output.get(&active.info.id))
            .into_iter()
            .flat_map(|output| &output.tools)
            .filter(|(_, tool)| tool.data.state.state != ToolCallState::Finished)
            .map(|(id, tool)| OwnedToolWork {
                tool_call_id: id.clone(),
                name: tool.data.state.name.clone(),
                state: tool.data.state.state,
            })
            .collect();
        Box::new(SessionOwnedWork {
            model: active.is_some_and(|active| {
                active.info.phase == Some(TurnPhase::Model) && active.info.outcome.is_none()
            }),
            tools,
            preparing: state.preparing,
            history_save: state.history_work.is_some(),
            metadata_save: state.record_work.is_some(),
            compaction: state.compaction.as_ref().map(|work| OwnedCompactionWork {
                source: work.activity.source.clone(),
                state: work.activity.state.clone(),
                cancellation_requested: work.activity.cancellation_requested,
            }),
            tool_cleanup: active.is_some_and(|active| {
                active.info.state == TurnState::Finalizing && active.tool_cleanup.pending()
            }),
            scope_cleanup: state.scope_cleanup_pending,
            result_handoff: state.handoff_pending,
        })
    }

    /// Read small fields under this owner's lock, without cloning configuration
    /// or history or retaining an execution binding beyond the read.
    pub(crate) fn summary(&self, caller: &CallerContext) -> Result<Option<SessionSummary>, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            if state.reclaimed {
                // The Agent may have copied this child just before actual
                // retirement. It no longer belongs in a current-object list.
                return Ok(None);
            }
            let control = Self::control_facts(&state);
            Ok(Some(SessionSummary {
                id: self.id.clone(),
                agent_id: self.agent_id.clone(),
                title: state.info.title.clone(),
                mode: state.info.mode,
                config_revision: state.info.config_revision,
                lifecycle: state.lifecycle,
                scheduling: control.scheduling,
                pause_reasons: control.pause_reasons,
                residency: state.residency,
                active_turn_id: state.active.as_ref().map(|active| active.info.id.clone()),
                next_step: state.next_step.len(),
                next_turn: state.next_turn.len(),
                pending_interactions: usize::from(
                    state
                        .active
                        .as_ref()
                        .is_some_and(|active| active.pending_interaction.is_some()),
                ),
                owned_work: Self::owned_work(&state),
                block_reason: control.block_reason,
                metadata_fault: state
                    .record_work
                    .as_ref()
                    .and_then(|work| work.fault.clone())
                    .or_else(|| state.close_fault.clone()),
                deletion: state
                    .deletion
                    .as_ref()
                    .map(|deletion| deletion.data.clone()),
                end_reason: state.end_reason.clone(),
            }))
        })
    }
}
