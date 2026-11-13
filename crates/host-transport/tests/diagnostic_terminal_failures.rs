//! Actual production Store inode refusal plus actual provider HTTP 401.
//! This is not an injected log writer failure, disk exhaustion or syscall errno.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_diagnostics::{LoggingState, Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_storage::{HistoryEntry, HistoryRecord};
use minicore_host_transport::HostClient;
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn records(path: &Path) -> Vec<Value> {
    // Read the actual file only after the writer has shut down and flushed.
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn matches(rows: &[Value], event: &str, turn: &TurnId) -> usize {
    rows.iter()
        .filter(|row| row["event"] == event && row["turn_id"] == turn.as_str())
        .count()
}
#[tokio::test(flavor = "current_thread")]
async fn primary_failure_is_recorded_before_save_repair_and_finished_only_after_real_barriers() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-terminal");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let (dispatch, guard) = minicore_host_diagnostics::prepare(logging).unwrap();
    let (dispatch, emitted) = observe_diagnostics(dispatch);
    tracing::dispatcher::set_global_default(dispatch).unwrap();
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
    let before = tokio::time::timeout(
        Duration::from_secs(2),
        emitted.wait_for(|records| {
            records
                .iter()
                .filter(|record| {
                    record["event"] == "turn.failed"
                        && record["context"]["turn_id"] == turn.as_str()
                })
                .count()
                == 1
        }),
    )
    .await
    .expect("primary owner ERROR must exist before repairing the original history save");
    assert!(!before.iter().any(|record| {
        record["event"] == "turn.finished" && record["context"]["turn_id"] == turn.as_str()
    }));
    // On this single-thread executor, the last cleanup publication and the
    // following driver_running=false publication have no await between them.
    // These work flags alone would not prove driver settlement across threads.
    let settled = tokio::time::timeout(WAIT, async {
        loop {
            let state = session.state(options()).await.unwrap();
            let work = &state.data.owned_work;
            if !state.data.saving && !work.model && !work.preparing && !work.tool_cleanup {
                break state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("original execution and save worker must return before explicit repair");
    assert_eq!(settled.data.active_turn_id.as_ref(), Some(&turn));
    assert!(settled.data.owned_work.history_save);
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
    tokio::time::timeout(WAIT, async {
        while session.state(options()).await.unwrap().data.scheduling != Scheduling::Auto {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stop(&host).await;
    let saved: Vec<HistoryRecord> = std::fs::read_to_string(&history)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(saved.len(), 2);
    assert!(matches!(&saved[0].entry, HistoryEntry::Input { turn_id, .. } if turn_id == &turn));
    assert_eq!(saved[0].position, 0);
    let terminal = &saved[1];
    assert_eq!(terminal.position, 1);
    let HistoryEntry::TurnFinished {
        turn_id,
        outcome,
        finished_at,
        requests,
        tool_rounds,
        model_attempts,
        ..
    } = &terminal.entry
    else {
        panic!("the original terminal record must be confirmed exactly once");
    };
    assert_eq!(turn_id, &turn);
    assert_eq!(*outcome, TurnOutcome::Failed);
    assert_eq!(*requests, 1);
    assert_eq!(*tool_rounds, 0);
    assert_eq!(model_attempts.as_slice(), &[1]);
    let info = finished.data.info.as_ref().unwrap();
    assert_eq!(info.history_position, Some(terminal.position));
    assert_eq!(info.recorded_at, Some(terminal.recorded_at));
    assert_eq!(info.finished_at, Some(*finished_at));
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let rows = records(&log);
    assert_eq!(matches(&rows, "turn.failed", &turn), 1);
    assert_eq!(matches(&rows, "turn.finished", &turn), 1);
    assert_eq!(matches(&rows, "session.history_write_failed", &turn), 1);
    let scheduling: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["event"] == "session.scheduling_changed"
                && row["session_id"] == target.session_id.as_str()
        })
        .collect();
    assert_eq!(
        scheduling
            .iter()
            .filter(|row| {
                matches!(
                    row["attributes"]["reason"].as_str(),
                    Some("execution_failed" | "finalizing")
                ) && row["attributes"]["execution_failure"] == true
            })
            .count(),
        1
    );
    for blocked in [true, false] {
        assert_eq!(
            scheduling
                .iter()
                .filter(|row| {
                    row["attributes"]["reason"] == "history_save_finished"
                        && row["attributes"]["blocked"] == blocked
                })
                .count(),
            1
        );
    }
    let resumed: Vec<_> = scheduling
        .iter()
        .filter(|row| {
            // Either real owner can confirm the last outstanding save. The
            // scheduling transition must still happen exactly once after both.
            matches!(
                row["attributes"]["reason"].as_str(),
                Some("save_resume_finished" | "metadata_save_finished")
            ) && row["attributes"]["execution_failure"] == false
        })
        .collect();
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0]["attributes"]["execution_failure"], false);
    assert_eq!(resumed[0]["attributes"]["blocked"], false);
    assert_eq!(resumed[0]["attributes"]["paused"], false);
    assert!(scheduling.iter().all(|row| row["level"] == "DEBUG" && row["attributes"]["controller_revision"].is_u64()));

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
    // Authentication failed before a response: the original unsaved input
    // append is repaired first, then the real terminal row can be accepted.
    assert_eq!(save["attributes"]["phase"], "append");
    let done = rows
        .iter()
        .find(|row| row["event"] == "turn.finished")
        .unwrap();
    assert_eq!(done["level"], "INFO");
    assert_eq!(done["fault_code"], 4204);
    assert_eq!(done["attributes"]["outcome"], "failed");
    assert_eq!(done["attributes"]["persistence"], "persisted");
    assert!(done["attributes"]["duration_ms"].is_u64());
    assert!(
        resumed[0]["attributes"]["controller_revision"]
            .as_u64()
            .unwrap()
            > done["attributes"]["controller_revision"].as_u64().unwrap()
    );
    for row in [primary, save, done] {
        assert_eq!(row["host_id"], finished.source.session.host_id.as_str());
        assert_eq!(row["agent_id"], target.agent_id.as_str());
        assert_eq!(row["session_id"], target.session_id.as_str());
        assert_eq!(row["turn_id"], turn.as_str());
        assert_eq!(
            row["load_id"],
            finished.source.load_id.as_ref().unwrap().as_str()
        );
    }
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
