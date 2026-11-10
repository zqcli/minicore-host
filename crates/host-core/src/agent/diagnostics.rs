//! Direct-owner diagnostic facts; no owner or message body enters a Record.
use super::*;
use crate::diagnostics::{Record, event};
use minicore_host_diagnostics::{Context, EventData, Scope};
use tracing::Level;
impl AgentOwner {
    pub(crate) fn session_creation_mode(
        &self,
        requested: Option<StorageMode>,
    ) -> Option<StorageMode> {
        self.state
            .lock()
            .expect("Agent mutex poisoned")
            .info
            .mode
            .session_mode(requested)
            .ok()
    }
    pub(crate) fn diagnostic_context(&self) -> Context {
        Context {
            host_id: Some(self.host_id.clone()),
            host_incarnation: Some(self.incarnation.clone()),
            agent_id: Some(self.id.clone()),
            ..Context::default()
        }
    }
    pub(super) fn lifecycle_record(
        &self,
        name: &'static str,
        phase: &'static str,
        mode: StorageMode,
        operation_id: Option<OperationId>,
    ) -> Record {
        Record::new(
            event(
                Level::INFO,
                Scope::Agent,
                "agent.owner",
                name,
                "Agent lifecycle boundary committed",
                &["mode", "phase"],
            ),
            EventData::default()
                .context(Context {
                    operation_id,
                    ..self.diagnostic_context()
                })
                .attribute("mode", mode_name(mode))
                .attribute("phase", phase),
        )
    }
    pub(crate) fn session_creation_record(
        &self,
        context: Context,
        mode: Option<StorageMode>,
        phase: &'static str,
        fault: Option<&Fault>,
    ) -> Record {
        let (level, name) = match fault {
            None => (Level::INFO, "session.created"),
            Some(fault) if fault.kind() == Some(FaultKind::SessionResourceMissing) => {
                (Level::WARN, "agent.resources_refused")
            }
            Some(fault)
                if matches!(
                    fault.kind(),
                    Some(
                        FaultKind::HostStorageUnavailable
                            | FaultKind::SessionHistoryWriteFailed
                            | FaultKind::AgentInternalUnexpected
                    )
                ) =>
            {
                (Level::ERROR, "session.creation_failed")
            }
            Some(_) => (Level::WARN, "session.creation_failed"),
        };
        let mut data = EventData::default()
            .context(context)
            .attribute("phase", phase);
        if let Some(mode) = mode {
            data = data.attribute("mode", mode_name(mode));
        }
        if let Some(fault) = fault {
            data = data.fault(fault);
        }
        Record::new(
            event(
                level,
                Scope::Agent,
                "agent.owner",
                name,
                "Direct Agent owner settled a Session creation boundary",
                &["mode", "phase"],
            ),
            data,
        )
    }
}
pub(super) fn mode_name(mode: StorageMode) -> &'static str {
    match mode {
        StorageMode::Memory => "memory",
        StorageMode::Persistent => "persistent",
    }
}
