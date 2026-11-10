//! Final disposition of actual resource preparation, before a Turn is bound.
use super::*;

impl SessionController {
    pub(in crate::session) fn resource_refused(
        &self,
        state: &SessionState,
        fault: &Fault,
    ) -> Record {
        let level = if matches!(
            fault.kind(),
            Some(
                FaultKind::SessionResourceMissing
                    | FaultKind::SessionConfigInvalid
                    | FaultKind::SessionConfigConflict
                    | FaultKind::SessionLifecycleClosing
                    | FaultKind::HostLifecycleUnavailable
            )
        ) {
            Level::WARN
        } else {
            Level::ERROR
        };
        Record::new(
            event(
                level,
                Scope::Session,
                "session.resources",
                "session.resource_refused",
                "Actual resource preparation ended before binding queued input",
                &[
                    "phase",
                    "config_revision",
                    "controller_revision",
                    "next_step_len",
                    "next_turn_len",
                ],
            ),
            EventData::default()
                .context(diagnostic::session_context(&self.identity()))
                .fault(fault)
                .attribute("phase", "preparing")
                .attribute("config_revision", state.info.config_revision)
                .attribute("controller_revision", state.controller_revision)
                .attribute("next_step_len", state.next_step.len() as u64)
                .attribute("next_turn_len", state.next_turn.len() as u64),
        )
    }
}
