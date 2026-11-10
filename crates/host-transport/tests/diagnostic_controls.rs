//! Real scheduling mutations and first cancellation claims, without observation listeners.
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
async fn scheduling_and_cancel_records_use_committed_facts_and_only_first_claims() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-controls");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let mut expected = Vec::new();
    for target_turn in [false, true] {
        let http = Http::start(vec![Reply::gated(Arc::new(Semaphore::new(0)))]).await;
        let host = host(&http.endpoint).await;
        let client = HostClient::in_process(host.clone()).unwrap();
        let created = client.create_agent(create(None), options()).await.unwrap();
        let target = target(&created);
        let session = client.session(target.clone());
        let pause = session
            .pause(
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(),
            )
            .await
            .unwrap();
        let repeat = session
            .pause(
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(),
            )
            .await
            .unwrap();
        let message = input();
        session.send(message.clone(), options()).await.unwrap();
        let unload = session
            .unload(
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(),
            )
            .await
            .unwrap();
        assert_eq!(unload.result.scheduling, Scheduling::Paused);
        assert_eq!(unload.result.residency, Residency::Unloaded);
        let queued = session.state(options()).await.unwrap();
        assert_eq!(queued.data.next_turn.len(), 1);
        assert!(queued.data.active_turn_id.is_none());
        assert_eq!(http.count(), 0);
        let load = session
            .load(
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(),
            )
            .await
            .unwrap();
        assert_eq!(load.result.residency, Residency::Loading);
        tokio::time::timeout(WAIT, async {
            while session.state(options()).await.unwrap().data.preparing {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let resume = session
            .resume(
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(),
            )
            .await
            .unwrap();
        assert_eq!(resume.result.scheduling, Scheduling::Auto);
        let turn = bound(&session, &message.message_id).await;
        tokio::time::timeout(WAIT, http.started.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        let operation = OperationId::new().unwrap();
        if target_turn {
            let receipt = session
                .cancel_turn(
                    TurnCancelRequest {
                        operation_id: operation.clone(),
                        turn_id: turn.clone(),
                        keep_inbox: true,
                    },
                    options(),
                )
                .await
                .unwrap();
            assert!(receipt.result.cancel_claimed);
            let again = session
                .cancel_turn(
                    TurnCancelRequest {
                        operation_id: OperationId::new().unwrap(),
                        turn_id: turn.clone(),
                        keep_inbox: true,
                    },
                    options(),
                )
                .await
                .unwrap();
            assert!(!again.result.cancel_claimed);
        } else {
            let receipt = session
                .cancel(
                    SessionCancelRequest {
                        operation_id: operation.clone(),
                        keep_inbox: true,
                    },
                    options(),
                )
                .await
                .unwrap();
            assert!(receipt.result.cancel_claimed);
            let again = session
                .cancel(
                    SessionCancelRequest {
                        operation_id: OperationId::new().unwrap(),
                        keep_inbox: true,
                    },
                    options(),
                )
                .await
                .unwrap();
            assert!(!again.result.cancel_claimed);
        }
        let done = session.wait_turn(turn.clone(), options()).await.unwrap();
        assert_eq!(
            done.data.info.unwrap().outcome,
            Some(TurnOutcome::Cancelled)
        );
        stop(&host).await;
        assert_eq!(http.count(), 1);
        http.abort().await;
        expected.push((
            target,
            turn,
            operation,
            [pause, unload, resume],
            repeat,
            load,
        ));
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
    for (target, turn, operation, changes, repeat, load) in expected {
        for (receipt, reason) in changes.iter().zip(["pause", "unload", "resume"]) {
            let events: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row["event"] == "session.scheduling_changed"
                        && row["operation_id"] == receipt.operation_id.as_str()
                })
                .collect();
            assert_eq!(events.len(), 1, "{reason}");
            let row = events[0];
            assert_eq!(row["session_id"], target.session_id.as_str());
            assert_eq!(row["attributes"]["reason"], reason);
            assert_eq!(
                row["attributes"]["controller_revision"],
                receipt.result.controller_revision
            );
            assert_eq!(
                row["attributes"]["next_turn_len"],
                u64::from(reason != "pause")
            );
            assert_eq!(row["level"], "DEBUG");
        }
        assert!(
            !rows
                .iter()
                .any(|row| row["event"] == "session.scheduling_changed"
                    && row["operation_id"] == repeat.operation_id.as_str())
        );
        let loads: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "session.scheduling_changed"
                    && row["operation_id"] == load.operation_id.as_str()
            })
            .collect();
        assert_eq!(loads.len(), 2);
        assert!(loads.iter().any(|row| row["attributes"]["reason"] == "load"
            && row["attributes"]["residency"] == "loading"));
        assert!(
            loads
                .iter()
                .any(|row| row["attributes"]["reason"] == "load_finished"
                    && row["attributes"]["residency"] == "loaded")
        );
        let claims: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == "turn.cancel_claimed" && row["turn_id"] == turn.as_str())
            .collect();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0]["operation_id"], operation.as_str());
        assert_eq!(claims[0]["session_id"], target.session_id.as_str());
        assert_eq!(claims[0]["attributes"]["reason"], "cancel");
        assert_eq!(claims[0]["level"], "INFO");
    }
    assert!(!rows.iter().any(|row| row["level"] == "ERROR"));
    for private in [
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        ERROR_BODY,
        directory.path().to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
}
