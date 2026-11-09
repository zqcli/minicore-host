use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::{Result, WorkspaceError};

/// Caller-owned cancellation and optional deadline. There is no implicit timeout.
#[derive(Clone, Debug, Default)]
pub struct QueryContext {
    pub cancellation: CancellationToken,
    pub deadline: Option<Instant>,
}

impl QueryContext {
    pub fn check(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(WorkspaceError::Cancelled)
        } else if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Err(WorkspaceError::Deadline)
        } else {
            Ok(())
        }
    }

    pub(crate) async fn stopped(&self) -> WorkspaceError {
        let deadline = async {
            if let Some(deadline) = self.deadline {
                tokio::time::sleep_until(deadline.into()).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => WorkspaceError::Cancelled,
            _ = deadline => WorkspaceError::Deadline,
        }
    }
}

pub(crate) async fn blocking<T, F>(context: QueryContext, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&QueryContext) -> Result<T> + Send + 'static,
{
    context.check()?;
    tokio::task::spawn_blocking(move || {
        context.check()?;
        work(&context)
    })
    .await
    .map_err(|_| WorkspaceError::WorkerFailed)?
}
