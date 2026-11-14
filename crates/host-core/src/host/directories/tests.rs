use super::*;
use crate::{HostConfig, PreparedHostConfiguration, ResourceFuture};
use std::{path::Path, sync::Condvar, time::Duration};
use tokio::sync::Semaphore;

pub(crate) struct DirectoryGate {
    entered: Semaphore,
    released: Mutex<bool>,
    release: Condvar,
}
impl DirectoryGate {
    pub(super) fn wait(&self) {
        self.entered.add_permits(1);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.release.wait(released).unwrap();
        }
    }
    fn open(&self) {
        *self.released.lock().unwrap() = true;
        self.release.notify_all();
    }
}
impl Drop for DirectoryGate {
    fn drop(&mut self) {
        self.open();
    }
}

struct UnusedResources;
impl ResourceResolver for UnusedResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "directory fixture has no model",
            ))
        })
    }
}
fn prepared(root: &Path) -> PreparedHostConfiguration {
    PreparedHostConfiguration::new(
        HostConfig {
            workspace_roots: vec![root.to_str().unwrap().into()],
            ..HostConfig::default()
        },
        Arc::new(UnusedResources),
        vec![],
        None,
    )
    .unwrap()
}
fn request(root: &Path, name: &str) -> DirectoryMkdirRequest {
    DirectoryMkdirRequest {
        operation_id: OperationId::new().unwrap(),
        parent: root.to_str().unwrap().into(),
        name: name.into(),
    }
}
fn query(root: &Path) -> DirectoryListRequest {
    DirectoryListRequest {
        path: root.to_str().unwrap().into(),
        hidden: false,
        cursor: None,
        limit: 10,
    }
}

#[tokio::test]
async fn host_authority_precedes_real_directory_reads_and_writes_without_sessions() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::memory_configured(prepared(root.path())).unwrap();
    let operator = host.connect().unwrap();
    let observer = host.connect().unwrap();
    assert_eq!(
        host.workspace_roots(&observer.caller())
            .unwrap()
            .roots
            .len(),
        1
    );
    assert!(
        host.workspace_list(&observer.caller(), query(root.path()))
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    let fault = host
        .workspace_mkdir(&observer.caller(), request(root.path(), "forbidden"))
        .await
        .unwrap_err();
    assert_eq!(fault.kind(), Some(FaultKind::HostAccessReadOnly));
    assert!(!root.path().join("forbidden").exists());
    let expired = operator.caller();
    operator.close();
    assert_eq!(
        host.workspace_roots(&expired).unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert_eq!(
        host.workspace_list(&expired, query(root.path()))
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert_eq!(
        host.workspace_mkdir(&expired, request(root.path(), "expired"))
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert_eq!(
        host.workspace_mkdir(&observer.caller(), request(root.path(), "still-read-only"))
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    let replacement = host.connect().unwrap();
    let receipt = host
        .workspace_mkdir(&replacement.caller(), request(root.path(), "created"))
        .await
        .unwrap();
    assert!(
        matches!(&receipt.result, DirectoryMkdirResult::Created { path } if Path::new(path).is_dir())
    );
    assert_eq!(
        host.receipt_get(&observer.caller(), &receipt.operation_id)
            .unwrap(),
        Receipt::WorkspaceMkdir(receipt)
    );
    assert!(host.state.lock().unwrap().agents.is_empty());
}

#[tokio::test]
async fn failed_write_receipts_and_duplicate_operation_ids_keep_separate_real_facts() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::memory_configured(prepared(root.path())).unwrap();
    let operator = host.connect().unwrap();
    let request = request(root.path(), "created");
    let operation = request.operation_id.clone();
    let first = host
        .workspace_mkdir(&operator.caller(), request.clone())
        .await
        .unwrap();
    let second = host
        .workspace_mkdir(&operator.caller(), request)
        .await
        .unwrap();
    assert!(matches!(first.result, DirectoryMkdirResult::Created { .. }));
    assert!(
        matches!(second.result, DirectoryMkdirResult::Failed { error } if error.kind() == Some(FaultKind::HostWorkspaceAlreadyExists))
    );
    assert_eq!(
        host.receipts(&operator.caller(), &operation).unwrap().len(),
        2
    );
    assert_eq!(
        host.receipt_get(&operator.caller(), &operation)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );
    let error = directory_fault(DirectoryError::Io(std::io::Error::other(
        "private filesystem diagnostic",
    )));
    assert_eq!(error.kind(), Some(FaultKind::HostWorkspaceIoFailed));
    assert!(!serde_json::to_string(&error).unwrap().contains("private"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_write_outlives_waiter_disconnect_and_force_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::memory_configured(prepared(root.path())).unwrap();
    let operator = host.connect().unwrap();
    let observer = host.connect().unwrap();
    let gate = Arc::new(DirectoryGate {
        entered: Semaphore::new(0),
        released: Mutex::new(false),
        release: Condvar::new(),
    });
    *host.directory_gate.lock().unwrap() = Some(gate.clone());
    let request = request(root.path(), "survives");
    let operation = request.operation_id.clone();
    let owner = host.clone();
    let caller = operator.caller();
    let task = tokio::spawn(async move { owner.workspace_mkdir(&caller, request).await });
    tokio::time::timeout(Duration::from_secs(5), gate.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    operator.close();
    let receipt = host.receipt_get(&observer.caller(), &operation).unwrap();
    assert!(matches!(
        receipt,
        Receipt::WorkspaceMkdir(DirectoryMkdirReceipt {
            result: DirectoryMkdirResult::Creating {},
            ..
        })
    ));
    let shutdown = host
        .process_control()
        .shutdown(HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        })
        .unwrap();
    assert_eq!(shutdown.result.lifecycle, HostLifecycle::Stopping);
    tokio::task::yield_now().await;
    assert_eq!(
        host.info(&observer.caller()).unwrap().lifecycle,
        HostLifecycle::Stopping
    );
    assert!(!root.path().join("survives").exists());
    gate.open();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.info(&observer.caller()).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(root.path().join("survives").is_dir());
    assert!(matches!(
        host.receipt_get(&observer.caller(), &operation).unwrap(),
        Receipt::WorkspaceMkdir(DirectoryMkdirReceipt {
            result: DirectoryMkdirResult::Created { .. },
            ..
        })
    ));
}

#[tokio::test]
async fn published_root_changes_apply_to_new_directory_requests() {
    let original = tempfile::tempdir().unwrap();
    let replacement = tempfile::tempdir().unwrap();
    let host = Host::memory_configured(prepared(original.path())).unwrap();
    let operator = host.connect().unwrap();
    assert!(matches!(
        host.reload_configuration(&operator.caller(), 0, prepared(replacement.path()))
            .await
            .unwrap(),
        HostReloadResult::Applied {
            config_revision: 1,
            ..
        }
    ));
    assert_eq!(
        host.workspace_list(&operator.caller(), query(original.path()))
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostWorkspaceOutsideRoots)
    );
    let receipt = host
        .workspace_mkdir(&operator.caller(), request(replacement.path(), "new-scope"))
        .await
        .unwrap();
    assert!(matches!(
        receipt.result,
        DirectoryMkdirResult::Created { .. }
    ));
}
