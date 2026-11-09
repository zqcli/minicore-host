//! The application owns output; a Host can only read its process-wide health.
use super::Host;
use minicore_host_diagnostics::{Health, LoggingState, LoggingStatus};

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
