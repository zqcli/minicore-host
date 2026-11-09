//! Storage calls run outside owner and connection locks. The narrow helpers do
//! not publish business state or interpret a successful read as a confirmed save.
use minicore_host_protocol::{Fault, FaultKind};
use minicore_host_storage::StorageError;

pub(crate) async fn disk<T: Send + 'static>(
    kind: FaultKind,
    action: impl FnOnce() -> Result<T, StorageError> + Send + 'static,
) -> Result<T, Fault> {
    tokio::task::spawn_blocking(action)
        .await
        .map_err(|_| Fault::new(kind, "storage worker did not return a confirmed result"))?
        .map_err(|error| Fault::new(kind, error.to_string()))
}
