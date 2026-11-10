//! Real persistent owner and Runtime cancellation during a gated HTTP request.
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
async fn force_upgrade_records_one_actual_claim_without_guessing_a_missing_operation() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let data = directory.path().join("data");
    let mut logging = Options::new("diagnostic-force");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let http = Http::start(vec![Reply::gated(Arc::new(Semaphore::new(0)))]).await;
    let host = persistent_host(&data, &http.endpoint).await;
    let client = HostClient::in_process(host.clone()).unwrap();
    let mut creation = create(None);
    creation.mode = StorageMode::Persistent;
    let created = client.create_agent(creation, options()).await.unwrap();
    let target = target(&created);
    let session = client.session(target.clone());
    let message = input();
    session.send(message.clone(), options()).await.unwrap();
    let turn = bound(&session, &message.message_id).await;
    tokio::time::timeout(WAIT, http.started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let control = host.process_control();
    let graceful = OperationId::new().unwrap();
    let first = control
        .shutdown(HostShutdownRequest {
            operation_id: graceful,
            strategy: HostShutdownStrategy::Graceful,
        })
        .unwrap();
    assert_eq!(first.result.lifecycle, HostLifecycle::Draining);
    let forced = control
        .shutdown(HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        })
        .unwrap();
    assert_eq!(forced.result.strategy, HostShutdownStrategy::Force);
    stop(&host).await;
    assert_eq!(http.count(), 1);
    http.abort().await;
    let path = data
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str())
        .join("history.jsonl");
    let saved: Vec<Value> = std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        saved
            .iter()
            .filter(|row| row["entry"]["kind"] == "turn_finished"
                && row["entry"]["data"]["turn_id"] == turn.as_str()
                && row["entry"]["data"]["outcome"] == "cancelled")
            .count(),
        1
    );
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let claims: Vec<_> = rows
        .iter()
        .filter(|row| row["event"] == "turn.cancel_claimed" && row["turn_id"] == turn.as_str())
        .collect();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0]["session_id"], target.session_id.as_str());
    assert_eq!(claims[0]["agent_id"], target.agent_id.as_str());
    assert_eq!(claims[0]["attributes"]["reason"], "host_force");
    assert_eq!(claims[0]["level"], "INFO");
    assert!(
        claims[0].get("operation_id").is_none(),
        "the child never received an operation; do not reuse the first graceful operation"
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row["event"] == "turn.finished"
                && row["turn_id"] == turn.as_str()
                && row["attributes"]["outcome"] == "cancelled")
            .count(),
        1
    );
    assert!(!rows.iter().any(|row| row["level"] == "ERROR"));
    for private in [
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        ERROR_BODY,
        PROVIDER_MODEL,
        directory.path().to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
}
