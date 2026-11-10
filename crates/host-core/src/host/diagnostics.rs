//! The application owns output; a Host can only read its process-wide health.
use super::Host;
use crate::diagnostics::{Record, event};
use minicore_host_diagnostics::{Context, EventData, Scope};
use minicore_host_diagnostics::{Health, LoggingState, LoggingStatus};
use minicore_host_protocol::*;
use tracing::Level;

impl Host {
    /// Bind the application's shared process-pipeline health once.
    ///
    /// This is a trusted embedding hook, not a client operation. It neither
    /// installs a subscriber nor transfers the writer guard. All Hosts bound
    /// to the same pipeline report the same process counters. Shutting down or
    /// dropping a Host never shuts down that pipeline.
    pub fn bind_logging_health(&self, health: Health) -> Result<(), Health> {
        self.logging_health.set(health)
    }

    pub(super) fn logging_status(&self) -> LoggingStatus {
        self.logging_health.get().map_or(
            LoggingStatus {
                status: LoggingState::Disabled,
                dropped_records: None,
                write_failures: None,
            },
            Health::snapshot,
        )
    }
}

impl Host {
    pub(super) fn diagnostic_context(&self) -> Context {
        Context {
            host_id: Some(self.id.clone()),
            host_incarnation: Some(self.incarnation.clone()),
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
                Scope::Host,
                "host.owner",
                name,
                "Host lifecycle boundary committed",
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
    pub(super) fn agent_created_record(&self, context: Context, mode: StorageMode) -> Record {
        Record::new(
            event(
                Level::INFO,
                Scope::Agent,
                "host.owner",
                "agent.created",
                "Agent published by its direct Host owner",
                &["mode", "phase"],
            ),
            EventData::default()
                .context(context)
                .attribute("mode", mode_name(mode))
                .attribute("phase", "published"),
        )
    }
    pub(super) fn agent_removed_record(&self, id: &AgentId, mode: StorageMode) -> Record {
        Record::new(
            event(
                Level::INFO,
                Scope::Host,
                "host.owner",
                "host.agent_removed",
                "Host removed its retired Agent from the direct owner index",
                &["mode", "phase"],
            ),
            EventData::default()
                .context(Context {
                    agent_id: Some(id.clone()),
                    ..self.diagnostic_context()
                })
                .attribute("mode", mode_name(mode))
                .attribute("phase", "removed"),
        )
    }
    pub(super) fn shutdown_failure_record(
        &self,
        phase: &'static str,
        mode: StorageMode,
        operation: &OperationId,
        fault: &Fault,
    ) -> Record {
        Record::new(
            event(
                Level::ERROR,
                Scope::Host,
                "host.owner",
                "host.shutdown_failed",
                "Host could not join its owned shutdown work",
                &["mode", "phase"],
            ),
            EventData::default()
                .context(Context {
                    operation_id: Some(operation.clone()),
                    ..self.diagnostic_context()
                })
                .fault(fault)
                .attribute("mode", mode_name(mode))
                .attribute("phase", phase),
        )
    }
    pub(super) fn creation_failure_record(
        &self,
        context: Context,
        mode: StorageMode,
        phase: &'static str,
        fault: &Fault,
        forwarded: bool,
    ) -> Record {
        let level = if forwarded {
            Level::DEBUG
        } else if matches!(
            fault.kind(),
            Some(
                FaultKind::HostStorageUnavailable
                    | FaultKind::HostInternalUnexpected
                    | FaultKind::AgentInternalUnexpected
            )
        ) {
            Level::ERROR
        } else {
            Level::WARN
        };
        Record::new(
            event(
                level,
                Scope::Host,
                "host.owner",
                if fault.kind() == Some(FaultKind::SessionResourceMissing) {
                    "host.resources_refused"
                } else {
                    "agent.creation_failed"
                },
                "Agent creation did not complete its requested phase",
                &["mode", "phase", "forwarded"],
            ),
            EventData::default()
                .context(context)
                .fault(fault)
                .attribute("mode", mode_name(mode))
                .attribute("phase", phase)
                .attribute("forwarded", forwarded),
        )
    }
}
fn mode_name(mode: StorageMode) -> &'static str {
    match mode {
        StorageMode::Memory => "memory",
        StorageMode::Persistent => "persistent",
    }
}
