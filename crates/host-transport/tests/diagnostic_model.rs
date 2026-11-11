//! Actual Runtime retry/usage from scripted loopback HTTP, without public observers.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::HostClient;
use serde_json::{Value, json};
use std::time::Duration;

fn limited() -> Reply {
    Reply {
        status: 429,
        gate: None,
        body: json!({"error":{"code":"rate_limit_exceeded","message":ERROR_BODY}}).to_string(),
    }
}
fn usage_reply() -> Reply {
    let body = Reply::success().body.replace("data: [DONE]\n\n", &format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":9,"total_tokens":21,
            "prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":3}}})
    ));
    Reply {
        body,
        ..Reply::success()
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_model_attempts_usage_and_source_measured_wait_are_turn_bound() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-model");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let mut completed = Vec::new();
    for mode in 0..3 {
        let http = Http::start(match mode {
            0 => vec![limited(), limited(), usage_reply()],
            1 => vec![Reply::success()],
            _ => vec![limited(), limited(), limited()],
        })
        .await;
        let host = host(&http.endpoint).await;
        let client = HostClient::in_process(host.clone()).unwrap();
        let message = input();
        let mut creation = create(Some(message.clone()));
        creation
            .config
            .session_defaults
            .execution
            .model_retry_attempts = 3;
        creation
            .config
            .session_defaults
            .execution
            .model_retry_base_delay_ms = 1;
        let created = client.create_agent(creation, options()).await.unwrap();
        let target = target(&created);
        let done = result(&client.session(target.clone()), &message.message_id).await;
        assert_eq!(
            done.data.info.as_ref().unwrap().model_attempts,
            if mode == 1 { vec![1] } else { vec![3] }
        );
        assert_eq!(
            done.data.info.as_ref().unwrap().outcome,
            Some(if mode == 2 {
                TurnOutcome::Failed
            } else {
                TurnOutcome::Completed
            })
        );
        if mode == 2 {
            assert_eq!(
                done.data
                    .info
                    .as_ref()
                    .unwrap()
                    .fault
                    .as_ref()
                    .unwrap()
                    .code(),
                4201
            );
        }
        stop(&host).await;
        assert_eq!(http.finish().await.len(), if mode == 1 { 1 } else { 3 });
        completed.push((mode, target, done.source.turn_id));
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
    for (mode, target, turn) in completed {
        let calls: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == "model.finished" && row["turn_id"] == turn.as_str())
            .collect();
        assert_eq!(calls.len(), 1);
        let call = calls[0];
        assert_eq!(call["session_id"], target.session_id.as_str());
        assert_eq!(call["agent_id"], target.agent_id.as_str());
        assert_eq!(call["attributes"]["attempt"], if mode == 1 { 1 } else { 3 });
        assert_eq!(call["attributes"]["request_index"], 0);
        assert_eq!(call["attributes"]["response_index"], 0);
        let transport: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "model.transport_finished"
                    && row["attributes"]["loop_id"] == call["attributes"]["loop_id"]
            })
            .collect();
        assert_eq!(transport.len(), if mode == 1 { 1 } else { 3 });
        let retries: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "model.retry_scheduled" && row["turn_id"] == turn.as_str()
            })
            .collect();
        assert_eq!(retries.len(), if mode == 1 { 0 } else { 2 });
        if mode != 1 {
            assert_eq!(retries[0]["level"], "WARN");
            assert_eq!(retries[0]["attributes"]["attempt"], 2);
            assert_eq!(retries[1]["level"], "DEBUG");
            assert_eq!(retries[1]["attributes"]["attempt"], 3);
            assert!(retries.iter().all(|row| row["fault_code"] == 4201));
        }
        let recovery: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == "model.recovered" && row["turn_id"] == turn.as_str())
            .collect();
        assert_eq!(recovery.len(), usize::from(mode == 0));
        if mode == 0 {
            assert_eq!(recovery[0]["level"], "INFO");
            assert_eq!(recovery[0]["attributes"]["attempt"], 3);
            let waited = recovery[0]["attributes"]["wait_duration_ms"]
                .as_u64()
                .unwrap();
            assert!(waited > 0);
            eprintln!("actual recovery: attempts=3, source wait_duration_ms={waited}");
            assert!(recovery[0]["attributes"].get("retry_after_ms").is_none());
            for (key, value) in [
                ("input_tokens", 10),
                ("output_tokens", 6),
                ("reasoning_tokens", 3),
                ("cache_read_tokens", 2),
                ("provider_total_tokens", 21),
            ] {
                assert_eq!(call["attributes"][key], value);
            }
        } else {
            for key in ["input_tokens", "output_tokens", "provider_total_tokens"] {
                assert!(call["attributes"].get(key).is_none());
            }
        }
        for row in &retries {
            // Scheduled retry facts are not the later measured wait outcome.
            for unknown in ["wait_duration_ms", "wait_ms", "retry_after_ms"] {
                assert!(row["attributes"].get(unknown).is_none());
            }
        }
        assert_eq!(
            rows.iter()
                .filter(|row| row["level"] == "ERROR" && row["turn_id"] == turn.as_str())
                .count(),
            usize::from(mode == 2)
        );
    }
    for private in [
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        ERROR_BODY,
        PROVIDER_MODEL,
    ] {
        assert!(!text.contains(private));
    }
}
