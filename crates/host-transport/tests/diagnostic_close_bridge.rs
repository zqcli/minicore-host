//! Actual owner close/delete admission across production Host and Runtime.
//! The loopback upstream is held pending; abort stops only that test fixture.
#![cfg(unix)]
mod diagnostic_support;
use diagnostic_support::*;
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::HostClient;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_and_delete_records_cross_all_admission_layers_once_with_the_original_operation() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-close");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let mut closed = Vec::new();
    for mode in 0..3 {
        let http = Http::start(vec![Reply::gated(Arc::new(Semaphore::new(0)))]).await;
        let host = host(&http.endpoint).await;
        let client = HostClient::in_process(host.clone()).unwrap();
        let message = input();
        let created = client
            .create_agent(create(Some(message.clone())), options())
            .await
            .unwrap();
        let target = target(&created);
        let session = client.session(target.clone());
        let turn = bound(&session, &message.message_id).await;
        tokio::time::timeout(WAIT, http.started.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        let operation = OperationId::new().unwrap();
        match mode {
            0 => {
                let receipt = session
                    .close(
                        SessionCloseRequest {
                            operation_id: operation.clone(),
                        },
                        options(),
                    )
                    .await
                    .unwrap();
                assert_eq!(receipt.operation_id, operation);
            }
            1 => {
                let receipt = session
                    .delete(
                        SessionDeleteRequest {
                            operation_id: operation.clone(),
                            close: true,
                        },
                        options(),
                    )
                    .await
                    .unwrap();
                assert_eq!(receipt.operation_id, operation);
            }
            _ => {
                let receipt = client
                    .delete_agent(
                        AgentRef {
                            host_id: created.host_id.clone(),
                            agent_id: target.agent_id.clone(),
                        },
                        AgentDeleteRequest {
                            operation_id: operation.clone(),
                            scope: AgentDeleteScope::Sessions {
                                session_ids: vec![target.session_id.clone()],
                                close: true,
                            },
                        },
                        options(),
                    )
                    .await
                    .unwrap();
                assert_eq!(receipt.operation_id, operation);
            }
        }
        stop(&host).await;
        assert_eq!(http.count(), 1);
        http.abort().await;
        closed.push((target, turn, operation));
    }
    let idle_host = host("http://127.0.0.1:1/v1/chat/completions").await;
    let idle_client = HostClient::in_process(idle_host.clone()).unwrap();
    let created = idle_client
        .create_agent(create(None), options())
        .await
        .unwrap();
    let idle = target(&created);
    idle_client
        .session(idle.clone())
        .close(
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap();
    stop(&idle_host).await;
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (target, turn, operation) in closed {
        for name in ["session.close_accepted", "turn.cancel_claimed"] {
            let events: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row["event"] == name && row["session_id"] == target.session_id.as_str()
                })
                .collect();
            assert_eq!(events.len(), 1, "{name}");
            assert_eq!(events[0]["operation_id"], operation.as_str());
            assert_eq!(events[0]["agent_id"], target.agent_id.as_str());
            if name == "turn.cancel_claimed" {
                assert_eq!(events[0]["turn_id"], turn.as_str());
                assert_eq!(events[0]["attributes"]["phase"], "cancelling");
            }
        }
        let finished: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == "turn.finished" && row["turn_id"] == turn.as_str())
            .collect();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0]["attributes"]["outcome"], "cancelled");
        let completed: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "session.closed" && row["session_id"] == target.session_id.as_str()
            })
            .collect();
        assert_eq!(completed.len(), 1, "active close completion");
        assert!(
            completed[0]["attributes"]["controller_revision"]
                .as_u64()
                .unwrap()
                >= finished[0]["attributes"]["controller_revision"]
                    .as_u64()
                    .unwrap()
        );
    }
    let events: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["event"] == "session.closed" && row["session_id"] == idle.session_id.as_str()
        })
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["attributes"]["mode"], "memory");
    for private in [KEY, BODY, ROLE, ANSWER, ERROR_BODY, FRAMEWORK] {
        assert!(!text.contains(private));
    }
}
