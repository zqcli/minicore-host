//! Read-only scalar comparison at actual scheduling mutation boundaries.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::session) struct Schedule {
    paused: bool,
    failure_paused: bool,
    unloaded_with_input: bool,
    residency: Residency,
    blocked: bool,
}
impl Schedule {
    pub(in crate::session) fn capture(state: &SessionState) -> Self {
        Self {
            paused: state.paused,
            failure_paused: state.failure_paused,
            unloaded_with_input: state.unloaded_with_input,
            residency: state.residency,
            blocked: state.block_reason.is_some(),
        }
    }
}
impl SessionController {
    pub(in crate::session) fn scheduling_record(
        &self,
        state: &SessionState,
        previous: Schedule,
        reason: &'static str,
        operation: Option<&OperationId>,
    ) -> Option<Record> {
        let current = Schedule::capture(state);
        if previous == current {
            return None;
        }
        Some(Record::new(
            event(
                Level::DEBUG,
                Scope::Session,
                "session.controller",
                "session.scheduling_changed",
                "Session owner published changed scheduling facts",
                &[
                    "reason",
                    "controller_revision",
                    "paused",
                    "execution_failure",
                    "unloaded_with_input",
                    "residency",
                    "blocked",
                    "next_step_len",
                    "next_turn_len",
                ],
            ),
            EventData::default()
                .context(Context {
                    operation_id: operation.cloned(),
                    load_id: state.load_id.clone(),
                    turn_id: state.active.as_ref().map(|active| active.info.id.clone()),
                    ..diagnostic::session_context(&self.identity())
                })
                .attribute("reason", reason)
                .attribute("controller_revision", state.controller_revision)
                .attribute("paused", current.paused)
                .attribute("execution_failure", current.failure_paused)
                .attribute("unloaded_with_input", current.unloaded_with_input)
                .attribute("blocked", current.blocked)
                .attribute(
                    "residency",
                    match current.residency {
                        Residency::Unloaded => "unloaded",
                        Residency::Loading => "loading",
                        Residency::Loaded => "loaded",
                        Residency::Unloading => "unloading",
                    },
                )
                .attribute("next_step_len", state.next_step.len() as u64)
                .attribute("next_turn_len", state.next_turn.len() as u64),
        ))
    }
}
