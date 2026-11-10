//! Actual Agent metadata writes, owned waits and explicit reconciliation.
use super::support::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_storage::AgentRecord;
use std::{path::Path, sync::Arc, time::Duration};
use tracing::instrument::WithSubscriber;

fn update(operation_id: OperationId) -> AgentUpdateRequest {
    AgentUpdateRequest {
        operation_id,
        expected_config_revision: 0,
        changes: AgentUpdateChanges {
            name: Some("private updated Agent name".into()),
            ..Default::default()
        },
    }
}
fn read_agent(path: &Path, agent: &AgentRef) -> AgentRecord {
    serde_json::from_slice(
        &std::fs::read(
            path.join("agents")
                .join(agent.agent_id.as_str())
                .join("agent.json"),
        )
        .unwrap(),
    )
    .unwrap()
}
async fn create(host: &Arc<Host>, caller: &CallerContext) -> (AgentRef, SessionRef) {
    let result = host
        .create_agent_async(caller, request(StorageMode::Persistent))
        .await
        .unwrap();
    let agent = AgentRef {
        host_id: result.host_id,
        agent_id: result.agent_id.unwrap(),
    };
    let session = SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: result.session_id.unwrap(),
    };
    (agent, session)
}

#[tokio::test]
async fn failed_record_save_and_explicit_reconcile_preserve_original_operation_and_revision() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let (agent, operation, fault) = async {
        let host = Host::open_persistent(directory.path(), settings(), resources())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let (agent, _) = create(&host, &caller).await;
        let path = directory
            .path()
            .join("agents")
            .join(agent.agent_id.as_str());
        // A real directory at the record file path rejects the production store read/write.
        std::fs::rename(path.join("agent.json"), path.join("original-agent.json")).unwrap();
        std::fs::create_dir(path.join("agent.json")).unwrap();
        let operation = OperationId::new().unwrap();
        let receipt = host
            .agent_update(&caller, &agent, update(operation.clone()))
            .await
            .unwrap();
        assert_eq!(
            receipt.result,
            MetadataUpdateResult::Saving { config_revision: 0 }
        );
        let fault = until(|| host.agent_info(&caller, &agent).unwrap().metadata_fault).await;
        assert_eq!(fault.kind(), Some(FaultKind::HostStorageUnavailable));
        let info = host.agent_info(&caller, &agent).unwrap();
        assert_eq!(info.name, "private Agent name");
        assert_eq!(info.config_revision, 0);
        std::fs::remove_dir(path.join("agent.json")).unwrap();
        std::fs::rename(path.join("original-agent.json"), path.join("agent.json")).unwrap();
        let recovered = host.reconcile_agent_record(&caller, &agent).await.unwrap();
        assert_eq!(recovered.config_revision, 1);
        assert_eq!(recovered.name, "private updated Agent name");
        assert!(recovered.metadata_fault.is_none());
        assert_eq!(
            read_agent(directory.path(), &agent).data.name,
            recovered.name
        );
        assert_eq!(
            host.receipt_get(&caller, &operation).unwrap(),
            Receipt::AgentUpdate(receipt)
        );
        shutdown(&host, &caller).await;
        (agent, operation, fault)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    assert!(
        !rows
            .iter()
            .any(|row| row.to_string().contains("private updated Agent name"))
    );
    let correlated: Vec<_> = rows
        .iter()
        .filter(|row| row["operation_id"] == operation.as_str())
        .collect();
    assert_eq!(
        correlated
            .iter()
            .filter(|row| row["event"] == "agent.metadata_save_accepted")
            .count(),
        2
    );
    let errors: Vec<_> = correlated
        .iter()
        .filter(|row| row["level"] == "ERROR")
        .collect();
    assert_eq!(errors.len(), 1, "{rows:?}");
    let failed = errors[0];
    assert_eq!(failed["event"], "agent.metadata_save_failed");
    assert_eq!(failed["fault_code"], fault.code());
    assert_eq!(failed["fault_type"], fault.type_name());
    assert!(failed.get("diagnostic_id").is_none());
    assert_eq!(failed["attributes"]["phase"], "failed");
    assert_eq!(failed["attributes"]["config_revision"], 0);
    assert!(failed["attributes"].get("persistence").is_none());
    let committed: Vec<_> = correlated
        .iter()
        .filter(|row| row["event"] == "agent.metadata_committed")
        .collect();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0]["attributes"]["config_revision"], 1);
    assert_eq!(committed[0]["attributes"]["persistence"], "persisted");
    for row in correlated {
        assert_eq!(row["host_id"], agent.host_id.as_str());
        assert_eq!(row["agent_id"], agent.agent_id.as_str());
        assert_eq!(row["attributes"]["mode"], "persistent");
        assert!(row.get("session_id").is_none());
    }
}

#[tokio::test]
async fn default_clear_failure_has_one_agent_error_with_original_close_operation() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let (operation, session, fault) = async {
        let host = Host::open_persistent(directory.path(), settings(), resources())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let (agent, session) = create(&host, &caller).await;
        let path = directory
            .path()
            .join("agents")
            .join(agent.agent_id.as_str());
        std::fs::rename(path.join("agent.json"), path.join("original-agent.json")).unwrap();
        std::fs::create_dir(path.join("agent.json")).unwrap();
        let operation = OperationId::new().unwrap();
        host.session_close(
            &caller,
            &session,
            SessionCloseRequest {
                operation_id: operation.clone(),
            },
        )
        .unwrap();
        let fault = until(|| {
            host.session_state(&caller, &session)
                .unwrap()
                .data
                .metadata_fault
        })
        .await;
        assert_eq!(fault.kind(), Some(FaultKind::HostStorageUnavailable));
        assert_eq!(
            host.session_state(&caller, &session)
                .unwrap()
                .data
                .lifecycle,
            SessionLifecycle::Closing
        );
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().metadata_fault,
            Some(fault.clone())
        );
        std::fs::remove_dir(path.join("agent.json")).unwrap();
        std::fs::rename(path.join("original-agent.json"), path.join("agent.json")).unwrap();
        host.reconcile_agent_record(&caller, &agent).await.unwrap();
        assert!(
            read_agent(directory.path(), &agent)
                .data
                .default_session_id
                .is_none()
        );
        // Continue only through the existing explicit close operation. No implicit retry.
        host.session_close(
            &caller,
            &session,
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
        until(|| {
            (host
                .session_state(&caller, &session)
                .unwrap()
                .data
                .lifecycle
                == SessionLifecycle::Closed)
                .then_some(())
        })
        .await;
        shutdown(&host, &caller).await;
        (operation, session, fault)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    let errors: Vec<_> = rows.iter().filter(|row| row["level"] == "ERROR").collect();
    assert_eq!(errors.len(), 1, "{rows:?}");
    assert_eq!(errors[0]["event"], "agent.metadata_save_failed");
    assert_eq!(errors[0]["scope"], "AGENT");
    assert_eq!(errors[0]["operation_id"], operation.as_str());
    assert_eq!(errors[0]["session_id"], session.session_id.as_str());
    assert_eq!(errors[0]["fault_code"], fault.code());
    let commits: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["event"] == "agent.metadata_committed" && row["operation_id"] == operation.as_str()
        })
        .collect();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["session_id"], session.session_id.as_str());
    assert_eq!(commits[0]["attributes"]["persistence"], "persisted");
}

#[test]
fn accepted_default_save_outlives_aborted_wait_and_revoked_connection() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let agent = runtime.block_on(
        async {
            let host = Host::open_persistent(directory.path(), settings(), resources())
                .await
                .unwrap();
            let connection = host.connect().unwrap();
            let caller = connection.caller();
            let (agent, _) = create(&host, &caller).await;
            let (release, blocked) = std::sync::mpsc::channel::<()>();
            let (entered, entering) = tokio::sync::oneshot::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = entered.send(());
                let _ = blocked.recv();
            });
            entering.await.unwrap();
            let owner = host.clone();
            let target = agent.clone();
            let requester = caller.clone();
            let waiter = tokio::spawn(
                async move {
                    owner
                        .set_default_session_async(&requester, &target, None)
                        .await
                }
                .with_current_subscriber(),
            );
            until(|| {
                host.agent_info(&caller, &agent)
                    .unwrap()
                    .metadata_saving
                    .then_some(())
            })
            .await;
            assert!(
                host.agent_info(&caller, &agent)
                    .unwrap()
                    .default_session_id
                    .is_some()
            );
            waiter.abort();
            assert!(waiter.await.unwrap_err().is_cancelled());
            connection.close();
            let observer = host.connect().unwrap();
            let caller = observer.caller();
            release.send(()).unwrap();
            blocker.await.unwrap();
            until(|| (!host.agent_info(&caller, &agent).unwrap().metadata_saving).then_some(()))
                .await;
            assert!(
                host.agent_info(&caller, &agent)
                    .unwrap()
                    .default_session_id
                    .is_none()
            );
            assert!(
                read_agent(directory.path(), &agent)
                    .data
                    .default_session_id
                    .is_none()
            );
            shutdown(&host, &caller).await;
            agent
        }
        .with_subscriber(dispatch),
    );
    // Business publication precedes diagnostic emission. Let the real owned
    // polls finish before closing the independent writer; Stopped is not a
    // promise that every diagnostic has already entered the output pipeline.
    runtime.shutdown_timeout(Duration::from_secs(2));
    let rows = records(&file, guard);
    for event in ["agent.metadata_save_accepted", "agent.metadata_committed"] {
        let matching: Vec<_> = rows.iter().filter(|row| row["event"] == event).collect();
        assert_eq!(matching.len(), 1, "{rows:?}");
        assert_eq!(matching[0]["agent_id"], agent.agent_id.as_str());
        assert!(matching[0].get("operation_id").is_none());
    }
}

#[tokio::test]
async fn uninstalled_or_closed_logging_does_not_change_confirmed_metadata() {
    for closed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let dispatch = if closed {
            let (_file, dispatch, guard) = logging();
            assert_eq!(
                guard.shutdown(Duration::from_secs(2)),
                minicore_host_diagnostics::ShutdownOutcome::Flushed
            );
            dispatch
        } else {
            tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default())
        };
        async {
            let host = Host::open_persistent(directory.path(), settings(), resources())
                .await
                .unwrap();
            let connection = host.connect().unwrap();
            let caller = connection.caller();
            let (agent, _) = create(&host, &caller).await;
            let receipt = host
                .agent_update(&caller, &agent, update(OperationId::new().unwrap()))
                .await
                .unwrap();
            assert_eq!(
                receipt.result,
                MetadataUpdateResult::Saving { config_revision: 0 }
            );
            until(|| {
                (host.agent_info(&caller, &agent).unwrap().config_revision == 1).then_some(())
            })
            .await;
            assert_eq!(
                read_agent(directory.path(), &agent).data.name,
                "private updated Agent name"
            );
            assert!(
                host.agent_info(&caller, &agent)
                    .unwrap()
                    .metadata_fault
                    .is_none()
            );
            shutdown(&host, &caller).await;
        }
        .with_subscriber(dispatch)
        .await;
    }
}

#[tokio::test]
async fn unavailable_metadata_resource_is_one_warning_without_a_save_or_new_revision() {
    let (file, dispatch, guard) = logging();
    let (agent, operation, fault, committed) = async {
        let host = Host::memory(resources()).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let created = host
            .create_agent_async(&caller, request(StorageMode::Memory))
            .await
            .unwrap();
        let agent = AgentRef {
            host_id: created.host_id,
            agent_id: created.agent_id.unwrap(),
        };
        let before = host.agent_info(&caller, &agent).unwrap();
        let mut candidate = before.config.session_defaults.clone();
        candidate.model = "missing/model".parse().unwrap();
        let operation = OperationId::new().unwrap();
        let fault = host
            .agent_update(
                &caller,
                &agent,
                AgentUpdateRequest {
                    operation_id: operation.clone(),
                    expected_config_revision: 0,
                    changes: AgentUpdateChanges {
                        config: AgentConfigPatch {
                            session_defaults: Some(candidate),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap_err();
        assert_eq!(fault.kind(), Some(FaultKind::SessionResourceMissing));
        let after = host.agent_info(&caller, &agent).unwrap();
        assert_eq!(after.config, before.config);
        assert_eq!(after.config_revision, before.config_revision);
        assert!(!after.metadata_saving);
        assert!(after.metadata_fault.is_none());
        assert_eq!(
            host.receipt_get(&caller, &operation).unwrap_err().kind(),
            Some(FaultKind::HostReceiptNotFound)
        );
        let committed = OperationId::new().unwrap();
        let receipt = host
            .agent_update(&caller, &agent, update(committed.clone()))
            .await
            .unwrap();
        assert_eq!(
            receipt.result,
            MetadataUpdateResult::Committed { config_revision: 1 }
        );
        shutdown(&host, &caller).await;
        (agent, operation, fault, committed)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    let matching: Vec<_> = rows
        .iter()
        .filter(|row| row["operation_id"] == operation.as_str())
        .collect();
    assert_eq!(matching.len(), 1, "{rows:?}");
    let row = matching[0];
    assert_eq!(row["level"], "WARN");
    assert_eq!(row["event"], "agent.resources_refused");
    assert_eq!(row["agent_id"], agent.agent_id.as_str());
    assert_eq!(row["attributes"]["mode"], "memory");
    assert_eq!(row["attributes"]["phase"], "resources");
    assert_eq!(row["fault_code"], fault.code());
    assert_eq!(row["fault_type"], fault.type_name());
    assert_eq!(row["diagnostic_id"], "original-resource-diagnostic");
    assert!(!rows.iter().any(|row| row["level"] == "ERROR"));
    let committed: Vec<_> = rows
        .iter()
        .filter(|row| row["operation_id"] == committed.as_str())
        .collect();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0]["event"], "agent.metadata_committed");
    assert_eq!(committed[0]["attributes"]["mode"], "memory");
    assert_eq!(committed[0]["attributes"]["config_revision"], 1);
    assert_eq!(committed[0]["attributes"]["persistence"], "not_applicable");
}

#[tokio::test]
async fn session_and_agent_delete_keep_original_operation_on_default_save_failure() {
    for cascade in [false, true] {
        let (file, dispatch, guard) = logging();
        let directory = tempfile::tempdir().unwrap();
        let (operation, session, fault) = async {
            let host = Host::open_persistent(directory.path(), settings(), resources())
                .await
                .unwrap();
            let connection = host.connect().unwrap();
            let caller = connection.caller();
            let (agent, session) = create(&host, &caller).await;
            let path = directory
                .path()
                .join("agents")
                .join(agent.agent_id.as_str());
            std::fs::rename(path.join("agent.json"), path.join("original-agent.json")).unwrap();
            std::fs::create_dir(path.join("agent.json")).unwrap();
            let operation = OperationId::new().unwrap();
            if cascade {
                host.agent_delete(
                    &caller,
                    &agent,
                    AgentDeleteRequest {
                        operation_id: operation.clone(),
                        scope: AgentDeleteScope::Sessions {
                            session_ids: vec![session.session_id.clone()],
                            close: true,
                        },
                    },
                )
                .unwrap();
            } else {
                host.session_delete(
                    &caller,
                    &session,
                    SessionDeleteRequest {
                        operation_id: operation.clone(),
                        close: true,
                    },
                )
                .unwrap();
            }
            let fault = until(|| {
                if cascade {
                    host.agent_info(&caller, &agent)
                        .unwrap()
                        .deletion
                        .and_then(|deletion| match deletion.data {
                            DataDeletion::Failed { error } => Some(error),
                            _ => None,
                        })
                } else {
                    host.session_state(&caller, &session)
                        .unwrap()
                        .data
                        .deletion
                        .and_then(|deletion| match deletion.data {
                            DataDeletion::Failed { error } => Some(error),
                            _ => None,
                        })
                }
            })
            .await;
            assert_eq!(fault.kind(), Some(FaultKind::HostStorageUnavailable));
            assert_eq!(
                host.agent_info(&caller, &agent).unwrap().metadata_fault,
                Some(fault.clone())
            );
            assert_eq!(
                host.session_state(&caller, &session)
                    .unwrap()
                    .data
                    .lifecycle,
                SessionLifecycle::Closing
            );
            assert!(
                path.join("sessions")
                    .join(session.session_id.as_str())
                    .join("session.json")
                    .exists()
            );
            // This failed deletion retains its real partial state. No automatic
            // reconcile or shutdown retry is added merely to finish the fixture.
            (operation, session, fault)
        }
        .with_subscriber(dispatch)
        .await;
        let rows = records(&file, guard);
        let errors: Vec<_> = rows.iter().filter(|row| row["level"] == "ERROR").collect();
        assert_eq!(errors.len(), 1, "{rows:?}");
        assert_eq!(errors[0]["event"], "agent.metadata_save_failed");
        assert_eq!(errors[0]["operation_id"], operation.as_str());
        assert_eq!(errors[0]["session_id"], session.session_id.as_str());
        assert_eq!(errors[0]["fault_code"], fault.code());
        assert!(
            !rows
                .iter()
                .any(|row| row["event"] == "agent.metadata_committed")
        );
    }
}
