use std::time::SystemTime;

use tokio::time::Instant;

use super::ModelError;

/// An actual model retry wait, associated with its enclosing request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRetryProgress {
    /// Total attempt number including the initial call, starting at two for retries.
    pub attempt: u8,
    pub max_attempts: u8,
    /// The preceding attempt's original structured failure.
    pub error: ModelError,
    /// Source wall-clock estimate for display, never used to schedule execution.
    pub retry_at: SystemTime,
    /// The monotonic target actually used by the driver's backoff wait.
    pub retry_deadline: Instant,
}
