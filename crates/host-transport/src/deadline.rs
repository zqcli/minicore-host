use minicore_host_protocol::{CallError, CallOptions};
use std::{future::Future, time::Duration};
use tokio::time::{Instant, timeout_at};

/// A single client-side budget, including serialization, writer queuing and
/// decoding. No Host method or wire parameters receive this deadline.
pub(crate) struct Deadline {
    at: Instant,
    milliseconds: u64,
}
impl Deadline {
    pub(crate) fn new(options: CallOptions) -> Result<Self, CallError> {
        let started = Instant::now();
        options.validate().map_err(|error| CallError::Protocol {
            message: error.to_string(),
        })?;
        let milliseconds = options.timeout_ms();
        let at = started
            .checked_add(Duration::from_millis(milliseconds))
            .ok_or_else(|| CallError::Protocol {
                message: "wait_timeout_ms exceeds the monotonic clock range".into(),
            })?;
        Ok(Self { at, milliseconds })
    }
    pub(crate) async fn wait<T>(
        self,
        operation: impl Future<Output = Result<T, CallError>>,
    ) -> Result<T, CallError> {
        let result = timeout_at(self.at, operation).await;
        // Also count synchronous encoding/decoding and direct-owner work within
        // the future's last poll, rather than resetting the timer at an await.
        if Instant::now() >= self.at {
            return Err(CallError::WaitTimeout {
                wait_timeout_ms: self.milliseconds,
            });
        }
        result.unwrap_or(Err(CallError::WaitTimeout {
            wait_timeout_ms: self.milliseconds,
        }))
    }
}
