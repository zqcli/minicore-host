//! Host-owned directory selection, independent of Agent and Session lifetimes.
use super::*;
use minicore_host_workspace::DirectoryError;
use std::panic::{AssertUnwindSafe, catch_unwind};

impl Host {
    pub fn workspace_roots(&self, caller: &CallerContext) -> Result<DirectoryRoots, Fault> {
        self.access.read(caller, || {
            self.configuration
                .snapshot()
                .directories
                .roots()
                .map_err(directory_fault)
        })
    }

    pub async fn workspace_list(
        &self,
        caller: &CallerContext,
        request: DirectoryListRequest,
    ) -> Result<DirectoryListResult, Fault> {
        let directories = self.access.read(caller, || {
            Ok(Arc::clone(&self.configuration.snapshot().directories))
        })?;
        let result = tokio::task::spawn_blocking(move || directories.list(request))
            .await
            .map_err(|_| directory_worker_fault())?
            .map_err(directory_fault)?;
        self.access.read(caller, || Ok(result))
    }

    /// Once admitted, the Host keeps the real write and its receipt until the
    /// blocking worker finishes, even if this caller stops waiting or disconnects.
    pub async fn workspace_mkdir(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: DirectoryMkdirRequest,
    ) -> Result<DirectoryMkdirReceipt, Fault> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "directory creation requires a live Tokio runtime",
            )
        })?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            if state.lifecycle != HostLifecycle::Running {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host is not accepting directory creation",
                ));
            }
            let directories = Arc::clone(&self.configuration.snapshot().directories);
            let mut receipt = DirectoryMkdirReceipt {
                kind: DirectoryMkdirKind::WorkspaceMkdir,
                host_id: self.id.clone(),
                host_incarnation: self.incarnation.clone(),
                operation_id: request.operation_id,
                result: DirectoryMkdirResult::Creating {},
            };
            let receipts = state
                .controls
                .entry(receipt.operation_id.clone())
                .or_default();
            let slot = receipts.len();
            receipts.push(Receipt::WorkspaceMkdir(receipt.clone()));
            state.directory_writes += 1;
            let owner = Arc::clone(self);
            runtime.spawn_blocking(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    #[cfg(test)]
                    if let Some(gate) = owner.directory_gate.lock().unwrap().clone() {
                        gate.wait();
                    }
                    directories
                        .mkdir(&request.parent, &request.name)
                        .map_err(directory_fault)
                }))
                .unwrap_or_else(|_| Err(directory_worker_fault()));
                receipt.result = match result {
                    Ok(path) => DirectoryMkdirResult::Created { path },
                    Err(error) => DirectoryMkdirResult::Failed { error },
                };
                {
                    let mut state = owner.state.lock().expect("Host mutex poisoned");
                    state
                        .controls
                        .get_mut(&receipt.operation_id)
                        .expect("accepted receipt")[slot] =
                        Receipt::WorkspaceMkdir(receipt.clone());
                    state.directory_writes -= 1;
                }
                owner.changed.notify_waiters();
                let _ = sender.send(receipt);
            });
            Ok(())
        })?;
        receiver.await.map_err(|_| directory_worker_fault())
    }
}

fn directory_worker_fault() -> Fault {
    Fault::new(
        FaultKind::HostInternalUnexpected,
        "directory worker did not finish",
    )
}
fn directory_fault(error: DirectoryError) -> Fault {
    let kind = match error {
        DirectoryError::InvalidPath => FaultKind::HostWorkspacePathInvalid,
        DirectoryError::OutsideRoots => FaultKind::HostWorkspaceOutsideRoots,
        DirectoryError::NotFound => FaultKind::HostWorkspaceNotFound,
        DirectoryError::NotDirectory => FaultKind::HostWorkspaceNotDirectory,
        DirectoryError::AlreadyExists => FaultKind::HostWorkspaceAlreadyExists,
        DirectoryError::InvalidRange => FaultKind::HostWorkspaceRangeInvalid,
        DirectoryError::InvalidCursor => FaultKind::HostWorkspaceCursorInvalid,
        DirectoryError::NonUtf8Path => FaultKind::HostWorkspaceUnsupported,
        DirectoryError::Io(_) => FaultKind::HostWorkspaceIoFailed,
    };
    Fault::new(kind, error.to_string())
}

#[cfg(test)]
pub(super) mod tests;
