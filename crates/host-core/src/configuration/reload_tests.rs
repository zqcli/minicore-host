//! Explicit scheduling test doubles around real configuration owner and storage.
use super::{tests::*, *};
use crate::{CallerContext, Host};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::sync::Semaphore;

struct TestPreparer {
    gate: Option<Arc<SaveGate>>,
    calls: AtomicUsize,
}
impl HostConfigurationPreparer for TestPreparer {
    fn prepare(&self, mut candidate: HostConfig) -> HostConfigurationFuture<'_> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                gate.entered.add_permits(1);
                gate.release.acquire().await.unwrap().forget();
            }
            if candidate.name == "prepare failure" {
                return Err(invalid("explicit preparation test failure"));
            }
            if candidate.name == "changed by preparer" {
                candidate.name = "wrong candidate".into();
            }
            Ok(prepared(candidate, "new-key"))
        })
    }
}
fn preparer(gate: Option<Arc<SaveGate>>) -> Arc<TestPreparer> {
    Arc::new(TestPreparer {
        gate,
        calls: AtomicUsize::new(0),
    })
}
fn preparation_gate() -> Arc<SaveGate> {
    Arc::new(SaveGate {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    })
}
/// Explicit unit fixture binds a real empty directory to an isolated owner.
/// Production startup is exercised separately through open_directory_configured.
pub(super) fn bind_directory(host: &Host) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    host.configuration_directory
        .set(directory.path().to_path_buf())
        .unwrap();
    write_candidate(host, &host.configuration.snapshot().config);
    directory
}
pub(super) fn write_candidate(host: &Host, candidate: &HostConfig) {
    let directory = host
        .configuration_directory
        .get()
        .expect("test directory bound");
    std::fs::write(
        directory.join("host.json"),
        serde_json::to_vec(candidate).unwrap(),
    )
    .unwrap();
}
fn request(
    host: &Host,
    candidate: HostConfig,
    expected_revision: ConfigRevision,
) -> HostReloadRequest {
    write_candidate(host, &candidate);
    HostReloadRequest {
        operation_id: OperationId::new().unwrap(),
        expected_revision,
    }
}
fn progress(host: &Host, caller: &CallerContext, operation: &OperationId) -> HostReloadReceipt {
    match host.receipt_get(caller, operation).unwrap() {
        Receipt::HostReload(receipt) => receipt,
        other => panic!("wrong owner facts: {other:?}"),
    }
}
async fn settled_reload(
    host: &Host,
    caller: &CallerContext,
    operation: &OperationId,
) -> HostReloadReceipt {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let receipt = progress(host, caller, operation);
            if !matches!(
                receipt.result,
                HostReloadResult::Preparing { .. } | HostReloadResult::Applying { .. }
            ) && !host.configuration.status().0
            {
                return receipt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn canonical_reload_owns_preparation_save_and_force_cleanup_after_waiter_disconnect() {
    let directory = tempfile::tempdir().unwrap();
    let prepare_gate = preparation_gate();
    let prep = preparer(Some(prepare_gate.clone()));
    let host = Host::open_configured(directory.path(), config("old"), |value| async {
        Ok(prepared(value, "old-key").with_preparer(prep.clone()))
    })
    .await
    .unwrap();
    let _configuration_directory = bind_directory(&host);
    let original = host.connect_network("old-key").unwrap();
    let old_reader = host.connect_network("old-key").unwrap();
    let observer = host.connect().unwrap();
    let save_gate = gate(&host);
    let request = request(&host, config("new"), 0);
    let operation = request.operation_id.clone();
    // The client wait is independently cancellable after Host::reload returns.
    let waiter = {
        let host = host.clone();
        let caller = original.caller();
        tokio::spawn(async move {
            let receipt = host.reload(&caller, request).unwrap();
            std::future::pending::<()>().await;
            receipt
        })
    };
    prepare_gate.entered.acquire().await.unwrap().forget();
    let formed = progress(&host, &observer.caller(), &operation);
    assert_eq!(
        formed.result,
        HostReloadResult::Preparing { config_revision: 0 }
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    original.close();
    let operator = host.connect().unwrap();
    assert!(operator.initial_info().can_operate);
    host.shutdown(
        &operator.caller(),
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    assert_eq!(
        host.info(&observer.caller()).unwrap().lifecycle,
        HostLifecycle::Stopping
    );
    assert_eq!(
        host.config_get(&observer.caller()).unwrap().config.name,
        "old"
    );
    prepare_gate.release.add_permits(1);
    save_gate.entered.acquire().await.unwrap().forget();
    assert_eq!(
        progress(&host, &observer.caller(), &operation).result,
        HostReloadResult::Applying { config_revision: 0 }
    );
    assert!(host.info(&old_reader.caller()).is_ok());
    assert_eq!(
        host.info(&operator.caller()).unwrap().lifecycle,
        HostLifecycle::Stopping
    );
    save_gate.release.add_permits(1);
    stopped(&host, &operator.caller()).await;
    assert_eq!(
        progress(&host, &observer.caller(), &operation).result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert_eq!(
        formed.result,
        HostReloadResult::Preparing { config_revision: 0 }
    );
    assert!(host.info(&old_reader.caller()).is_err());
    assert!(host.info(&observer.caller()).is_ok());
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("host.json")).unwrap())
            .unwrap();
    assert_eq!(stored["data"]["config"]["name"], "new");
    assert_eq!(stored["data"]["config_revision"], 1);
    // The initialization-only assembler is released at the actual stop barrier.
    assert!(host.configuration.state.lock().unwrap().preparer.is_none());
}

#[tokio::test]
async fn canonical_preparation_failures_remain_queryable_after_later_success() {
    let prep = preparer(None);
    let host =
        Host::memory_configured(prepared(config("old"), "old-key").with_preparer(prep.clone()))
            .unwrap();
    let _configuration_directory = bind_directory(&host);
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let network = host.connect_network("old-key").unwrap();
    let mut failures = Vec::new();
    for name in ["prepare failure", "changed by preparer", ""] {
        let request = request(&host, config(name), 0);
        let operation = request.operation_id.clone();
        let formed = host.reload(&caller, request).unwrap();
        let failed = settled_reload(&host, &caller, &operation).await;
        assert!(
            matches!(failed.result, HostReloadResult::Failed { config_revision: 0, ref error } if error.kind() == Some(FaultKind::HostConfigInvalid))
        );
        assert!(matches!(formed.result, HostReloadResult::Preparing { .. }));
        assert_eq!(host.config_get(&caller).unwrap().config.name, "old");
        assert!(host.info(&network.caller()).is_ok());
        failures.push(failed);
    }
    let request = request(&host, config("new"), 0);
    let operation = request.operation_id.clone();
    host.reload(&caller, request).unwrap();
    assert_eq!(
        settled_reload(&host, &caller, &operation).await.result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    for failed in failures {
        assert_eq!(progress(&host, &caller, &failed.operation_id), failed);
    }
    assert_eq!(prep.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn canonical_reload_rejects_before_acceptance_and_never_rebinds_old_authority() {
    let prepare_gate = preparation_gate();
    let prep = preparer(Some(prepare_gate.clone()));
    let host =
        Host::memory_configured(prepared(config("old"), "old-key").with_preparer(prep.clone()))
            .unwrap();
    let _configuration_directory = bind_directory(&host);
    let original = host.connect().unwrap();
    let reader = host.connect().unwrap();
    for (caller, revision, kind) in [
        (reader.caller(), 0, FaultKind::HostAccessReadOnly),
        (original.caller(), 1, FaultKind::HostConfigConflict),
    ] {
        let request = request(&host, config("new"), revision);
        let operation = request.operation_id.clone();
        assert_eq!(
            host.reload(&caller, request).unwrap_err().kind(),
            Some(kind)
        );
        assert_eq!(
            host.receipt_get(&reader.caller(), &operation)
                .unwrap_err()
                .kind(),
            Some(FaultKind::HostReceiptNotFound)
        );
    }
    assert_eq!(prep.calls.load(Ordering::SeqCst), 0);
    let accepted = host
        .reload(&original.caller(), request(&host, config("first"), 0))
        .unwrap();
    prepare_gate.entered.acquire().await.unwrap().forget();
    let contender = request(&host, config("competing"), 0);
    let contender_operation = contender.operation_id.clone();
    assert_eq!(
        host.reload(&original.caller(), contender)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostConfigConflict)
    );
    assert!(
        host.receipts(&reader.caller(), &contender_operation)
            .unwrap()
            .is_empty()
    );
    let old_caller = original.caller();
    original.close();
    let next = host.connect().unwrap();
    assert!(next.initial_info().can_operate);
    assert!(
        host.reload(&old_caller, request(&host, config("old connection"), 0))
            .is_err()
    );
    host.shutdown(
        &next.caller(),
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    assert_eq!(
        host.reload(&next.caller(), request(&host, config("too late"), 0))
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
    prepare_gate.release.add_permits(1);
    stopped(&host, &next.caller()).await;
    assert_eq!(
        progress(&host, &next.caller(), &accepted.operation_id).result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert_eq!(prep.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn canonical_mixed_restart_rejects_every_change_but_unchanged_restart_fields_do_not_block() {
    let prep = preparer(None);
    let mut initial = config("old");
    initial.network.listen = Some("127.0.0.1:4321".parse().unwrap());
    initial.logging.level = LogLevel::Debug;
    let host =
        Host::memory_configured(prepared(initial.clone(), "old-key").with_preparer(prep.clone()))
            .unwrap();
    let _configuration_directory = bind_directory(&host);
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let network = host.connect_network("old-key").unwrap();
    let mut mixed = initial.clone();
    mixed.name = "new".into();
    mixed.network.listen = Some("127.0.0.1:4322".parse().unwrap());
    mixed.logging.level = LogLevel::Trace;
    let accepted = host.reload(&caller, request(&host, mixed, 0)).unwrap();
    let receipt = settled_reload(&host, &caller, &accepted.operation_id).await;
    assert_eq!(
        receipt.result,
        HostReloadResult::RestartRequired {
            config_revision: 0,
            fields: vec![RestartField::NetworkListen, RestartField::Logging]
        }
    );
    assert_eq!(progress(&host, &caller, &receipt.operation_id), receipt);
    assert_eq!(host.config_get(&caller).unwrap().config, initial);
    assert!(host.info(&network.caller()).is_ok());
    assert_eq!(prep.calls.load(Ordering::SeqCst), 1);
    initial.name = "valid hot change".into();
    let hot = host
        .reload(&caller, request(&host, initial.clone(), 0))
        .unwrap();
    assert_eq!(
        settled_reload(&host, &caller, &hot.operation_id)
            .await
            .result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert_eq!(host.config_get(&caller).unwrap().config, initial);
    assert!(host.info(&network.caller()).is_err());
}

#[tokio::test]
async fn canonical_actual_save_failure_retains_failed_facts_and_original_live_key() {
    let directory = tempfile::tempdir().unwrap();
    let prep = preparer(None);
    let host = Host::open_configured(directory.path(), config("old"), |value| async {
        Ok(prepared(value, "old-key").with_preparer(prep))
    })
    .await
    .unwrap();
    let _configuration_directory = bind_directory(&host);
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let network = host.connect_network("old-key").unwrap();
    let path = directory.path().join("host.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let formed = host
        .reload(&caller, request(&host, config("new"), 0))
        .unwrap();
    let failed = settled_reload(&host, &caller, &formed.operation_id).await;
    assert!(
        matches!(failed.result, HostReloadResult::Failed { config_revision: 0, ref error } if error.kind() == Some(FaultKind::HostStorageUnavailable))
    );
    assert_eq!(host.config_get(&caller).unwrap().config.name, "old");
    assert!(host.info(&network.caller()).is_ok());
    assert!(host.connect_network("new-key").is_err());
    assert_eq!(
        host.reload(&caller, request(&host, config("no implicit retry"), 0))
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostConfigConflict)
    );
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let info = host.info(&caller).unwrap();
            if let Some(fault) = info.shutdown_fault {
                assert_eq!(fault.kind(), Some(FaultKind::HostStorageUnavailable));
                assert_eq!(info.lifecycle, HostLifecycle::Stopping);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(progress(&host, &caller, &formed.operation_id), failed);
    assert!(matches!(formed.result, HostReloadResult::Preparing { .. }));
}

#[tokio::test]
async fn canonical_key_rotation_keeps_local_calls_and_duplicate_ids_are_not_replays() {
    let prep = preparer(None);
    let host =
        Host::memory_configured(prepared(config("old"), "old-key").with_preparer(prep.clone()))
            .unwrap();
    let _configuration_directory = bind_directory(&host);
    let original = host.connect_network("old-key").unwrap();
    let local_reader = host.connect().unwrap();
    let request = request(&host, config("first"), 0);
    let operation = request.operation_id.clone();
    let formed = host.reload(&original.caller(), request).unwrap();
    assert_eq!(
        settled_reload(&host, &local_reader.caller(), &operation)
            .await
            .result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert!(host.info(&original.caller()).is_err());
    let new_connection = host.connect_network("new-key").unwrap();
    assert!(new_connection.initial_info().can_operate);
    assert_eq!(
        progress(&host, &new_connection.caller(), &operation).operation_id,
        operation
    );
    write_candidate(&host, &config("second"));
    let second = host
        .reload(
            &new_connection.caller(),
            HostReloadRequest {
                operation_id: operation.clone(),
                expected_revision: 1,
            },
        )
        .unwrap();
    assert!(matches!(
        second.result,
        HostReloadResult::Preparing { config_revision: 1 }
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while host
            .config_get(&local_reader.caller())
            .unwrap()
            .config_revision
            != 2
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        host.receipt_get(&new_connection.caller(), &operation)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostReceiptAmbiguous)
    );
    assert_eq!(
        host.receipts(&local_reader.caller(), &operation)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        host.config_get(&local_reader.caller()).unwrap().config.name,
        "second"
    );
    assert_eq!(prep.calls.load(Ordering::SeqCst), 2);
    assert!(matches!(
        formed.result,
        HostReloadResult::Preparing { config_revision: 0 }
    ));
    // Same actual key in the second publication did not revoke the connection.
    assert!(host.info(&new_connection.caller()).is_ok());
}

struct CleanupGate {
    entered: Semaphore,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}
/// A failed assertion must still release the synchronous resource destructor
/// before this test's runtime waits for its accepted configuration work.
struct CleanupRelease(Arc<CleanupGate>);
impl Drop for CleanupRelease {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}
struct CleanupResources(Arc<CleanupGate>);
impl ResourceResolver for CleanupResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async { Err(invalid("cleanup fixture does not execute Turns")) })
    }
}
impl Drop for CleanupResources {
    fn drop(&mut self) {
        self.0.entered.add_permits(1);
        let mut released = self.0.released.lock().unwrap();
        while !*released {
            released = self.0.wake.wait(released).unwrap();
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn canonical_publication_and_force_wait_for_real_resource_drop_outside_locks() {
    let cleanup = Arc::new(CleanupGate {
        entered: Semaphore::new(0),
        released: std::sync::Mutex::new(false),
        wake: std::sync::Condvar::new(),
    });
    let initial = PreparedHostConfiguration::new(
        config("old"),
        Arc::new(CleanupResources(cleanup.clone())),
        vec![],
        Some("old-key".into()),
    )
    .unwrap()
    .with_preparer(preparer(None));
    let host = Host::memory_configured(initial).unwrap();
    let release = CleanupRelease(cleanup.clone());
    let _configuration_directory = bind_directory(&host);
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let formed = host
        .reload(&caller, request(&host, config("new"), 0))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), cleanup.entered.acquire())
        .await
        .expect("resource cleanup must start after publication")
        .unwrap()
        .forget();
    assert_eq!(host.config_get(&caller).unwrap().config_revision, 1);
    assert_eq!(
        progress(&host, &caller, &formed.operation_id).result,
        HostReloadResult::Applying { config_revision: 1 }
    );
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    assert_eq!(
        host.info(&caller).unwrap().lifecycle,
        HostLifecycle::Stopping
    );
    drop(release);
    stopped(&host, &caller).await;
    assert_eq!(
        progress(&host, &caller, &formed.operation_id).result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
}

#[tokio::test]
async fn unassembled_host_does_not_claim_reload_is_available() {
    let host = Host::memory_configured(prepared(config("old"), "old-key")).unwrap();
    let _configuration_directory = bind_directory(&host);
    let connection = host.connect().unwrap();
    let request = request(&host, config("new"), 0);
    let operation = request.operation_id.clone();
    assert_eq!(
        host.reload(&connection.caller(), request)
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostMethodUnavailable)
    );
    assert!(
        host.receipts(&connection.caller(), &operation)
            .unwrap()
            .is_empty()
    );
}

struct ReplacingPreparer {
    calls: AtomicUsize,
    replacement: Arc<TestPreparer>,
}
impl HostConfigurationPreparer for ReplacingPreparer {
    fn prepare(&self, config: HostConfig) -> HostConfigurationFuture<'_> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(prepared(config, "new-key").with_preparer(self.replacement.clone()))
        })
    }
}
#[tokio::test]
async fn reload_candidates_cannot_replace_the_initial_application_preparer() {
    let replacement = preparer(None);
    let original = Arc::new(ReplacingPreparer {
        calls: AtomicUsize::new(0),
        replacement: replacement.clone(),
    });
    let host =
        Host::memory_configured(prepared(config("old"), "old-key").with_preparer(original.clone()))
            .unwrap();
    let _configuration_directory = bind_directory(&host);
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    for revision in 0..2 {
        let accepted = host
            .reload(&caller, request(&host, config("new"), revision))
            .unwrap();
        assert_eq!(
            settled_reload(&host, &caller, &accepted.operation_id)
                .await
                .result,
            HostReloadResult::Applied {
                config_revision: revision + 1,
                agents: Vec::new()
            }
        );
    }
    assert_eq!(original.calls.load(Ordering::SeqCst), 2);
    assert_eq!(replacement.calls.load(Ordering::SeqCst), 0);
}
