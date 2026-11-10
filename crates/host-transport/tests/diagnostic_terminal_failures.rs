//! Actual production Store inode refusal plus actual provider HTTP 401.
//! This is not an injected log writer failure, disk exhaustion or syscall errno.
#![cfg(unix)]
mod diagnostic_support;
use diagnostic_support::*;
use minicore_host_diagnostics::{LoggingState, Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::HostClient;
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn records(path: &Path) -> Vec<Value> {
    // The file writer can be between two writes. This observer reads only
    // complete diagnostic lines; it does not decide any business state.
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
fn matches(rows: &[Value], event: &str, turn: &TurnId) -> usize {
    rows.iter()
        .filter(|row| row["event"] == event && row["turn_id"] == turn.as_str())
        .count()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn primary_failure_is_recorded_before_save_repair_and_finished_only_after_real_barriers() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-terminal");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let health = guard.health();
    let gate = Arc::new(Semaphore::new(0));
    let http = Http::start(vec![Reply {
        gate: Some(gate.clone()),
        ..Reply::auth_failure()
    }])
    .await;
    let root = directory.path().join("data");
    let host = persistent_host(&root, &http.endpoint).await;
    assert!(host.bind_logging_health(health.clone()).is_ok());
    let client = HostClient::in_process(host.clone()).unwrap();
    let mut request = create(None);
    request.mode = StorageMode::Persistent;
    let created = client.create_agent(request, options()).await.unwrap();
    let target = target(&created);
    let session = client.session(target.clone());
    let message = input();
    session.send(message.clone(), options()).await.unwrap();
    let turn = bound(&session, &message.message_id).await;
    tokio::time::timeout(WAIT, async {
        while http.count() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let history = root
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str())
        .join("history.jsonl");
    let original = history.with_extension("original");
    // Production Journal has an actual open file identity. Swap its path while
    // HTTP is gated so the next real save fails the existing identity check.
    std::fs::rename(&history, &original).unwrap();
    std::fs::write(&history, b"").unwrap();
    gate.add_permits(1);
    let failed = tokio::time::timeout(WAIT, async {
        loop {
            let info = session.turn(turn.clone(), options()).await.unwrap();
            if info.data.state.state == TurnState::Finalizing
                && info.data.state.persistence == PersistenceState::Failed
            {
                break info;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(failed.data.state.outcome, Some(TurnOutcome::Failed));
    assert_eq!(failed.data.state.fault.as_ref().unwrap().code(), 4204);
    assert_eq!(
        failed.data.state.persistence_fault.as_ref().unwrap().code(),
        3301
    );
    assert_eq!(health.snapshot().status, LoggingState::Ok);
    tokio::time::timeout(Duration::from_secs(2), async {
        while matches(&records(&log), "turn.failed", &turn) != 1 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("primary owner ERROR must exist before repairing the original history save");
    let before = records(&log);
    assert_eq!(matches(&before, "turn.finished", &turn), 0);
    // Restore exactly the original inode and use only the existing explicit
    // same-save continuation. No task is resent and no model is called again.
    std::fs::remove_file(&history).unwrap();
    std::fs::rename(&original, &history).unwrap();
    session
        .resume(
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap();
    let finished = session.wait_turn(turn.clone(), options()).await.unwrap();
    assert_eq!(
        finished.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    assert_eq!(
        finished.data.info.as_ref().unwrap().fault,
        failed.data.state.fault
    );
    assert_eq!(
        finished.data.info.as_ref().unwrap().persistence,
        PersistenceState::Persisted
    );
    assert!(
        finished
            .data
            .info
            .as_ref()
            .unwrap()
            .persistence_fault
            .is_none()
    );
    assert_eq!(http.count(), 1);
    stop(&host).await;
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let rows = records(&log);
    assert_eq!(matches(&rows, "turn.failed", &turn), 1);
    assert_eq!(matches(&rows, "turn.finished", &turn), 1);
    assert_eq!(matches(&rows, "session.history_write_failed", &turn), 1);
    let primary = rows
        .iter()
        .find(|row| row["event"] == "turn.failed")
        .unwrap();
    assert_eq!(primary["level"], "ERROR");
    assert_eq!(primary["fault_code"], 4204);
    assert_eq!(primary["attributes"]["outcome"], "failed");
    let save = rows
        .iter()
        .find(|row| row["event"] == "session.history_write_failed")
        .unwrap();
    assert_eq!(save["level"], "ERROR");
    assert_eq!(save["fault_code"], 3301);
    let done = rows
        .iter()
        .find(|row| row["event"] == "turn.finished")
        .unwrap();
    assert_eq!(done["level"], "INFO");
    assert_eq!(done["fault_code"], 4204);
    assert_eq!(done["attributes"]["outcome"], "failed");
    assert_eq!(done["attributes"]["persistence"], "persisted");
    assert!(done["attributes"]["duration_ms"].is_u64());
    assert_eq!(
        rows.iter()
            .filter(|row| row["level"] == "ERROR" && row["fault_code"] == 4204)
            .count(),
        1
    );
    let text = std::fs::read_to_string(&log).unwrap();
    for private in [
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        ERROR_BODY,
        PROVIDER_MODEL,
        history.to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
    http.finish().await;
}
