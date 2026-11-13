//! Real Store validation failures, not injected logger failures or disk exhaustion.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_core::{MessageBinding, SessionSnapshot};
use minicore_host_diagnostics::{LoggingState, Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::{HostClient, SessionClient};
use serde_json::Value;
use std::time::Duration;

async fn blocked(session: &SessionClient) -> SessionSnapshot {
    tokio::time::timeout(WAIT, async {
        loop {
            let snapshot = session.state(options()).await.unwrap();
            if !snapshot.data.preparing && snapshot.data.block_reason.is_some() {
                break snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_cold_load_and_metadata_failures_keep_business_state_and_record_only_actual_attempts()
{
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-storage");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let health = guard.health();
    let http = Http::start(vec![Reply::success(), Reply::success()]).await;
    let root = directory.path().join("data");
    let seed = persistent_host(&root, &http.endpoint).await;
    let client = HostClient::in_process(seed.clone()).unwrap();
    let first = input();
    let mut creation = create(Some(first.clone()));
    creation.mode = StorageMode::Persistent;
    let created = client.create_agent(creation, options()).await.unwrap();
    let target = target(&created);
    result(&client.session(target.clone()), &first.message_id).await;
    stop(&seed).await;
    drop(client);
    drop(seed);
    let session_path = root
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str());
    let history = session_path.join("history.jsonl");
    let original = std::fs::read(&history).unwrap();
    let mut corrupt = original.clone();
    corrupt.extend_from_slice(b"PRIVATE_CORRUPT_HISTORY\n");
    std::fs::write(&history, corrupt).unwrap();
    let host = persistent_host(&root, &http.endpoint).await;
    let client = HostClient::in_process(host.clone()).unwrap();
    let session = client.session(target.clone());
    let before = session.state(options()).await.unwrap();
    let cold = session
        .history(
            HistoryReadRequest {
                direction: minicore_host_protocol::HistoryDirection::Forward,
                cursor: None,
                limit: 10,
            },
            options(),
        )
        .await
        .unwrap_err();
    let CallError::Business(cold_fault) = cold else {
        panic!("expected actual storage Fault")
    };
    assert_eq!(cold_fault.kind(), Some(FaultKind::SessionHistoryLoadFailed));
    let after = session.state(options()).await.unwrap();
    assert_eq!(
        before.data.controller_revision,
        after.data.controller_revision
    );
    assert!(after.data.active_turn_id.is_none());
    assert!(after.data.load_id.is_none());
    assert!(after.data.block_reason.is_none());
    let message = input();
    session.send(message.clone(), options()).await.unwrap();
    let failed = blocked(&session).await;
    assert_eq!(failed.data.next_turn.len(), 1);
    assert!(failed.data.active_turn_id.is_none());
    assert_eq!(
        failed.data.block_reason.unwrap().fault.unwrap().kind(),
        Some(FaultKind::SessionHistoryLoadFailed)
    );
    assert!(matches!(
        session
            .message(message.message_id.clone(), options())
            .await
            .unwrap()
            .data
            .binding,
        MessageBinding::Queued
    ));
    assert_eq!(http.count(), 1);
    let load_operation = OperationId::new().unwrap();
    session
        .load(
            SessionControlRequest {
                operation_id: load_operation.clone(),
            },
            options(),
        )
        .await
        .unwrap();
    blocked(&session).await;
    assert_eq!(http.count(), 1);
    std::fs::write(&history, original).unwrap();
    session
        .resume(
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap();
    let finished = result(&session, &message.message_id).await;
    assert_eq!(
        finished.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(http.count(), 2);

    // The terminal history and its activity metadata have independent saves.
    // Finish the accepted activity checkpoint before breaking the config save.
    tokio::time::timeout(WAIT, async {
        loop {
            let state = session.state(options()).await.unwrap();
            assert!(state.data.metadata_fault.is_none());
            if !state.data.metadata_saving {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("accepted activity metadata must settle before the config failure");
    let metadata = session_path.join("session.json");
    let saved = session_path.join("session.original");
    std::fs::rename(&metadata, &saved).unwrap();
    std::fs::create_dir(&metadata).unwrap();
    let before = session.info(options()).await.unwrap();
    let update_operation = OperationId::new().unwrap();
    session
        .update(
            SessionUpdateRequest {
                operation_id: update_operation.clone(),
                expected_config_revision: before.config_revision,
                changes: SessionUpdateChanges {
                    title: NullablePatch::Set("PRIVATE_NEW_TITLE".into()),
                    ..Default::default()
                },
            },
            options(),
        )
        .await
        .unwrap();
    let fault = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(fault) = session.state(options()).await.unwrap().data.metadata_fault {
                break fault;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fault.kind(), Some(FaultKind::SessionHistoryWriteFailed));
    assert_eq!(
        session.info(options()).await.unwrap().config_revision,
        before.config_revision
    );
    assert_eq!(session.info(options()).await.unwrap().title, before.title);
    assert_eq!(health.snapshot().status, LoggingState::Ok);
    std::fs::remove_dir(&metadata).unwrap();
    std::fs::rename(&saved, &metadata).unwrap();
    session
        .resume(
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap();
    tokio::time::timeout(WAIT, async {
        while session.info(options()).await.unwrap().title.as_deref() != Some("PRIVATE_NEW_TITLE") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    session
        .close(
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap();
    tokio::time::timeout(WAIT, async {
        while session.state(options()).await.unwrap().data.lifecycle != SessionLifecycle::Closed {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stop(&host).await;
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let read: Vec<_> = rows
        .iter()
        .filter(|row| row["event"] == "session.history_read_failed")
        .collect();
    assert_eq!(read.len(), 3);
    for row in &read {
        assert_eq!(row["fault_code"], cold_fault.code());
        assert_eq!(row["level"], "ERROR");
        assert_eq!(row["session_id"], target.session_id.as_str());
        assert!(row.get("turn_id").is_none());
    }
    assert_eq!(
        read.iter()
            .filter(|row| row["attributes"]["phase"] == "cold_read")
            .count(),
        1
    );
    assert_eq!(
        read.iter()
            .filter(|row| row["operation_id"] == load_operation.as_str())
            .count(),
        1
    );
    let write: Vec<_> = rows
        .iter()
        .filter(|row| row["event"] == "session.metadata_save_failed")
        .collect();
    assert_eq!(write.len(), 1);
    assert_eq!(write[0]["fault_code"], fault.code());
    assert_eq!(write[0]["level"], "ERROR");
    assert_eq!(write[0]["session_id"], target.session_id.as_str());
    assert_eq!(write[0]["operation_id"], update_operation.as_str());
    assert_eq!(
        rows.iter()
            .filter(|row| row["event"] == "session.closed"
                && row["session_id"] == target.session_id.as_str())
            .count(),
        1
    );
    for private in [
        KEY,
        BODY,
        ROLE,
        ANSWER,
        FRAMEWORK,
        "PRIVATE_CORRUPT_HISTORY",
        "PRIVATE_NEW_TITLE",
        history.to_str().unwrap(),
        metadata.to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
    http.finish().await;
}
