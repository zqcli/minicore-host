//! Real Session metadata refusals retain their initiating operation across resume.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::{HostClient, SessionClient};
use serde_json::Value;
use std::time::Duration;

async fn failed(session: &SessionClient) {
    tokio::time::timeout(WAIT, async {
        loop {
            let state = session.state(options()).await.unwrap();
            if !state.data.metadata_saving
                && let Some(fault) = state.data.metadata_fault
            {
                assert_eq!(fault.kind(), Some(FaultKind::SessionHistoryWriteFailed));
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_and_close_save_errors_keep_original_operation_not_the_resume_operation() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-metadata-operation");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let mut expected = Vec::new();
    for (closing, paused) in [(false, false), (false, true), (true, false)] {
        let root = directory.path().join(if closing {
            "close-data"
        } else if paused {
            "paused-update-data"
        } else {
            "update-data"
        });
        let host = persistent_host(&root, "http://127.0.0.1:1/v1/chat/completions").await;
        let client = HostClient::in_process(host.clone()).unwrap();
        let mut creation = create(None);
        creation.mode = StorageMode::Persistent;
        let created = client.create_agent(creation, options()).await.unwrap();
        let target = target(&created);
        let session = client.session(target.clone());
        if paused {
            session
                .pause(
                    SessionControlRequest {
                        operation_id: OperationId::new().unwrap(),
                    },
                    options(),
                )
                .await
                .unwrap();
            assert_eq!(
                session.state(options()).await.unwrap().data.scheduling,
                Scheduling::Paused
            );
        }
        let metadata = root
            .join("agents")
            .join(target.agent_id.as_str())
            .join("sessions")
            .join(target.session_id.as_str())
            .join("session.json");
        let original = metadata.with_extension("original");
        std::fs::rename(&metadata, &original).unwrap();
        std::fs::create_dir(&metadata).unwrap();
        let operation = OperationId::new().unwrap();
        if closing {
            session
                .close(
                    SessionCloseRequest {
                        operation_id: operation.clone(),
                    },
                    options(),
                )
                .await
                .unwrap();
        } else {
            session
                .update(
                    SessionUpdateRequest {
                        operation_id: operation.clone(),
                        expected_config_revision: 0,
                        changes: SessionUpdateChanges {
                            title: NullablePatch::Set("PRIVATE_METADATA_TITLE".into()),
                            ..Default::default()
                        },
                    },
                    options(),
                )
                .await
                .unwrap();
        }
        failed(&session).await;
        let resume = OperationId::new().unwrap();
        session
            .resume(
                SessionControlRequest {
                    operation_id: resume.clone(),
                },
                options(),
            )
            .await
            .unwrap();
        failed(&session).await;
        // This second original save attempt is explicit, not an automatic retry.
        assert_eq!(session.info(options()).await.unwrap().config_revision, 0);
        std::fs::remove_dir(&metadata).unwrap();
        std::fs::rename(&original, &metadata).unwrap();
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
            loop {
                let state = session.state(options()).await.unwrap();
                if closing && state.data.lifecycle == SessionLifecycle::Closed {
                    break;
                }
                if !closing
                    && session.info(options()).await.unwrap().title.as_deref()
                        == Some("PRIVATE_METADATA_TITLE")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if !closing {
            session
                .close(
                    SessionCloseRequest {
                        operation_id: OperationId::new().unwrap(),
                    },
                    options(),
                )
                .await
                .unwrap();
        }
        stop(&host).await;
        expected.push((target, operation, resume, closing, paused));
    }
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (target, operation, resume, closing, paused) in expected {
        let failures: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "session.metadata_save_failed"
                    && row["session_id"] == target.session_id.as_str()
            })
            .collect();
        assert_eq!(failures.len(), 2);
        if !closing {
            let resumed: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row["event"] == "session.scheduling_changed"
                        && row["operation_id"] == operation.as_str()
                        && row["attributes"]["reason"] == "metadata_save_finished"
                })
                .collect();
            assert_eq!(resumed.len(), usize::from(paused));
            if paused {
                assert_eq!(resumed[0]["attributes"]["paused"], false);
                assert_eq!(resumed[0]["attributes"]["blocked"], false);
                assert_eq!(resumed[0]["level"], "DEBUG");
            }
        }
        for row in failures {
            assert_eq!(row["operation_id"], operation.as_str());
            assert_ne!(row["operation_id"], resume.as_str());
            assert!(row.get("turn_id").is_none());
            assert_eq!(row["fault_code"], 3301);
            assert_eq!(row["level"], "ERROR");
        }
    }
    for private in [
        KEY,
        ROLE,
        FRAMEWORK,
        "PRIVATE_METADATA_TITLE",
        directory.path().to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
}
