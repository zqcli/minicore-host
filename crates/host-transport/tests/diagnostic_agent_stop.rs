//! The actual Agent stop traverses its existing child cancellation boundary.
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
async fn agent_stop_records_only_its_first_actual_child_claim_with_original_operation() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-agent-stop");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let http = Http::start(vec![Reply::gated(Arc::new(Semaphore::new(0)))]).await;
    let host = host(&http.endpoint).await;
    let client = HostClient::in_process(host.clone()).unwrap();
    let created = client.create_agent(create(None), options()).await.unwrap();
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
    let operation = OperationId::new().unwrap();
    let receipt = client
        .stop_agent(
            AgentRef {
                host_id: created.host_id.clone(),
                agent_id: target.agent_id.clone(),
            },
            AgentStopRequest {
                operation_id: operation.clone(),
            },
            options(),
        )
        .await
        .unwrap();
    assert!(matches!(
        receipt.result.lifecycle,
        AgentStopLifecycle::Stopping | AgentStopLifecycle::Stopped
    ));
    let done = session.wait_turn(turn.clone(), options()).await.unwrap();
    assert_eq!(done.data.info.unwrap().outcome, Some(TurnOutcome::Cancelled));
    let repeat = OperationId::new().unwrap();
    client
        .stop_agent(
            AgentRef {
                host_id: created.host_id.clone(),
                agent_id: target.agent_id.clone(),
            },
            AgentStopRequest {
                operation_id: repeat.clone(),
            },
            options(),
        )
        .await
        .unwrap();
    stop(&host).await;
    assert_eq!(http.count(), 1);
    http.abort().await;
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let claims: Vec<_> = rows
        .iter()
        .filter(|row| row["event"] == "turn.cancel_claimed" && row["turn_id"] == turn.as_str())
        .collect();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0]["operation_id"], operation.as_str());
    assert_eq!(claims[0]["agent_id"], target.agent_id.as_str());
    assert_eq!(claims[0]["session_id"], target.session_id.as_str());
    assert_eq!(claims[0]["level"], "INFO");
    assert_eq!(claims[0]["attributes"]["reason"], "agent_disabled");
    assert!(
        claims[0]["attributes"]["controller_revision"]
            .as_u64()
            .is_some()
    );
    assert!(
        !rows
            .iter()
            .any(|row| row["event"] == "turn.cancel_claimed"
                && row["operation_id"] == repeat.as_str())
    );
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
