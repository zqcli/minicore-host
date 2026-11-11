//! Actual direct clients, production owners, Runtime, local HTTP and OS pipes.
//! These tests do not claim the yet-unrestored owner ERROR/lifecycle coverage.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_diagnostics::{LoggingState, Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::{HostClient, serve_stdio};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Semaphore,
};

fn rows<'a>(records: &'a [Value], component: &str, method: &str) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|row| row["component"] == component && row["attributes"]["method"] == method)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_typed_and_rpc_calls_keep_original_metadata_and_never_duplicate_the_summary() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("calls.jsonl");
    let mut logging = Options::new("reconstructed-transport");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(path.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let health = guard.health();
    let gate = Arc::new(Semaphore::new(0));
    let http = Http::start(vec![
        Reply::success(),
        Reply::auth_failure(),
        Reply::gated(gate.clone()),
        Reply::success(),
    ])
    .await;
    let host = host(&http.endpoint).await;
    assert!(host.bind_logging_health(health.clone()).is_ok());
    let local = HostClient::in_process(host.clone()).unwrap();
    let host_id = local.initial_info().host_id.clone();
    let incarnation = local.initial_info().host_incarnation.clone();
    let local_message = input();
    let local_request = create(Some(local_message.clone()));
    let local_operation = local_request.operation_id.clone();
    let local_created = local.create_agent(local_request, options()).await.unwrap();
    let local_target = target(&local_created);
    let local_session = local.session(local_target.clone());
    let completed = result(&local_session, &local_message.message_id).await;
    assert_eq!(
        completed.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    local_session.state(options()).await.unwrap();
    local.close();

    let (remote, server) = pipes(host.clone()).await;
    assert!(remote.initial_info().can_operate);
    let remote_message = input();
    let remote_request = create(Some(remote_message.clone()));
    let remote_operation = remote_request.operation_id.clone();
    let remote_created = remote
        .create_agent(remote_request, options())
        .await
        .unwrap();
    let remote_target = target(&remote_created);
    let remote_session = remote.session(remote_target.clone());
    let failed = result(&remote_session, &remote_message.message_id).await;
    assert_eq!(
        failed.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    assert_eq!(
        failed
            .data
            .info
            .as_ref()
            .unwrap()
            .fault
            .as_ref()
            .unwrap()
            .code(),
        4204
    );
    remote_session.state(options()).await.unwrap();
    assert_eq!(http.count(), 2);

    // Real invalid raw wire input keeps valid correlation leaves without
    // changing the original protocol refusal or entering a model call.
    let (mut send, input_pipe) = crate::os_pipe::pipe().await.unwrap();
    let (output_pipe, receive) = crate::os_pipe::pipe().await.unwrap();
    let raw_server = tokio::spawn(serve_stdio(host.clone(), input_pipe, output_pipe));
    let mut receive = BufReader::new(receive);
    let rejected = input();
    for (id, method, params, expected) in [
        (1, "initialize", json!({}), None),
        (
            2,
            "message.send",
            json!({"target":{"kind":"session","agent_id":remote_target.agent_id,"session_id":"PRIVATE_INVALID_TARGET"},"message_id":rejected.message_id,"in_reply_to":rejected.in_reply_to,"mode":"follow_up","text":BODY}),
            Some(-32602),
        ),
        (
            3,
            "PRIVATE_UNKNOWN_METHOD",
            json!({"private":BODY}),
            Some(-32601),
        ),
    ] {
        let wire =
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string() + "\n";
        send.write_all(wire.as_bytes()).await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(WAIT, receive.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], id);
        if let Some(code) = expected {
            assert_eq!(reply["error"]["code"], code);
        } else {
            assert!(reply.get("result").is_some());
        }
    }
    drop(send);
    raw_server.await.unwrap().unwrap();
    assert_eq!(http.count(), 2);

    let observer = HostClient::in_process(host.clone()).unwrap();
    assert!(!observer.initial_info().can_operate);
    let denied = input();
    let refusal = observer
        .session(remote_target.clone())
        .send(denied.clone(), options())
        .await
        .unwrap_err();
    let CallError::Business(refusal) = refusal else {
        panic!("expected original owner Fault")
    };
    assert_eq!(refusal.kind(), Some(FaultKind::HostAccessReadOnly));
    assert_eq!(http.count(), 2);

    // A real owner task continues after its RPC wait times out. Logging
    // records the wait result, not a cancelled/failed Turn or automatic replay.
    // The preceding real failed Turn correctly paused scheduling. Explicitly
    // resume through the existing public control before the next test task.
    remote_session
        .resume(
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap();
    let pending = input();
    let pending_receipt = remote_session
        .send(pending.clone(), options())
        .await
        .unwrap();
    let turn = bound(&remote_session, &pending.message_id).await;
    tokio::time::timeout(WAIT, async {
        while http.count() != 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut abandoned = Box::pin(remote_session.wait_turn(turn.clone(), options()));
    tokio::select! {
        biased;
        result = &mut abandoned => panic!("gated Turn unexpectedly completed: {}", result.is_ok()),
        _ = tokio::time::sleep(Duration::from_millis(1)) => {},
    }
    drop(abandoned);
    let error = remote_session
        .wait_turn(
            turn.clone(),
            CallOptions {
                wait_timeout_ms: Some(5),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CallError::WaitTimeout { wait_timeout_ms: 5 }
    ));
    gate.add_permits(1);
    let resumed = remote_session
        .wait_turn(turn.clone(), options())
        .await
        .unwrap();
    assert_eq!(
        resumed.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(http.count(), 3);

    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&path).unwrap();
    let records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let accepted: Vec<_> = records
        .iter()
        .filter(|row| row["event"] == "session.input_accepted")
        .collect();
    let bound_inputs: Vec<_> = records
        .iter()
        .filter(|row| row["event"] == "session.input_bound")
        .collect();
    let turns: Vec<_> = records
        .iter()
        .filter(|row| row["event"] == "turn.started")
        .collect();
    assert_eq!(accepted.len(), 3);
    assert_eq!(bound_inputs.len(), 3);
    assert_eq!(turns.len(), 3);
    for (message, source) in [
        (&local_message, &completed.source),
        (&remote_message, &failed.source),
        (&pending, &resumed.source),
    ] {
        let input: Vec<_> = accepted
            .iter()
            .filter(|row| row["message_id"] == message.message_id.as_str())
            .collect();
        let binding: Vec<_> = bound_inputs
            .iter()
            .filter(|row| row["message_id"] == message.message_id.as_str())
            .collect();
        let started: Vec<_> = turns
            .iter()
            .filter(|row| row["turn_id"] == source.turn_id.as_str())
            .collect();
        assert_eq!(input.len(), 1);
        assert_eq!(binding.len(), 1);
        assert_eq!(started.len(), 1);
        for row in [input[0], binding[0]] {
            assert_eq!(row["host_id"], source.session.host_id.as_str());
            assert_eq!(row["session_id"], source.session.session_id.as_str());
            assert_eq!(
                row["in_reply_to"],
                message.in_reply_to.as_ref().unwrap().as_str()
            );
            assert_eq!(row["attributes"]["queue"], "next-turn");
            assert_eq!(row["attributes"]["next_step_len"], 0);
        }
        assert!(input[0].get("turn_id").is_none());
        assert_eq!(input[0]["attributes"]["next_turn_len"], 1);
        assert_eq!(binding[0]["attributes"]["next_turn_len"], 0);
        assert_eq!(binding[0]["turn_id"], source.turn_id.as_str());
        assert_eq!(
            binding[0]["load_id"],
            source.load_id.as_ref().unwrap().as_str()
        );
        assert_eq!(
            binding[0]["attributes"]["controller_revision"],
            started[0]["attributes"]["controller_revision"]
        );
        assert!(
            binding[0]["attributes"]["controller_revision"]
                .as_u64()
                .unwrap()
                > input[0]["attributes"]["controller_revision"]
                    .as_u64()
                    .unwrap()
        );
    }
    let pending_row = accepted
        .iter()
        .find(|row| row["message_id"] == pending.message_id.as_str())
        .unwrap();
    assert_eq!(
        pending_row["attributes"]["controller_revision"],
        pending_receipt.result.controller_revision
    );
    let starts: Vec<_> = records
        .iter()
        .filter(|row| row["component"] == "session.runtime" && row["event"] == "model.started")
        .collect();
    assert_eq!(starts.len(), 3);
    for source in [&completed.source, &failed.source, &resumed.source] {
        let started: Vec<_> = starts
            .iter()
            .filter(|row| row["turn_id"] == source.turn_id.as_str())
            .collect();
        assert_eq!(started.len(), 1);
        let started = started[0];
        assert_eq!(started["scope"], "RUNTIME");
        assert_eq!(started["host_id"], source.session.host_id.as_str());
        assert_eq!(
            started["host_incarnation"],
            source.session.host_incarnation.as_str()
        );
        assert_eq!(started["agent_id"], source.session.agent_id.as_str());
        assert_eq!(started["session_id"], source.session.session_id.as_str());
        assert_eq!(
            started["load_id"],
            source.load_id.as_ref().unwrap().as_str()
        );
        assert_eq!(started["attributes"]["response_index"], 0);
        assert_eq!(started["attributes"]["request_index"], 0);
        // Match the actual provider attempt rather than inventing a new link.
        assert_eq!(
            records
                .iter()
                .filter(|row| row["event"] == "model.transport_started"
                    && row["attributes"]["loop_id"] == started["attributes"]["loop_id"]
                    && row["attributes"]["request_index"] == started["attributes"]["request_index"])
                .count(),
            1
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
        "PRIVATE_INVALID_TARGET",
        "PRIVATE_UNKNOWN_METHOD",
        &http.endpoint,
    ] {
        assert!(!text.contains(private), "diagnostic exposed private data");
    }
    for row in records
        .iter()
        .filter(|row| row["event"] == "rpc.call_finished")
    {
        assert_eq!(row["level"], "DEBUG");
        assert!(row["attributes"]["duration_ms"].is_u64());
        if row["scope"] == "CLIENT" {
            for field in [
                "host_id",
                "host_incarnation",
                "agent_id",
                "session_id",
                "turn_id",
                "tool_call_id",
                "load_id",
            ] {
                assert!(row.get(field).is_none());
            }
        }
    }
    for (operation, message) in [
        (&local_operation, &local_message),
        (&remote_operation, &remote_message),
    ] {
        let calls: Vec<_> = rows(&records, "client.rpc", "agent.create")
            .into_iter()
            .filter(|row| row["operation_id"] == operation.as_str())
            .collect();
        assert_eq!(calls.len(), 1);
        let row = calls[0];
        assert_eq!(row["message_id"], message.message_id.as_str());
        assert_eq!(
            row["in_reply_to"],
            message.in_reply_to.as_ref().unwrap().as_str()
        );
        assert_eq!(row["attributes"]["target_host_id"], host_id.as_str());
        assert_eq!(
            row["attributes"]["target_host_incarnation"],
            incarnation.as_str()
        );
        assert!(row["attributes"].get("target_agent_id").is_none());
        assert!(row["attributes"].get("target_session_id").is_none());
    }
    let server_creation = rows(&records, "host.rpc", "agent.create");
    assert_eq!(server_creation.len(), 1);
    assert_eq!(
        server_creation[0]["operation_id"],
        remote_operation.as_str()
    );
    assert_eq!(
        server_creation[0]["message_id"],
        remote_message.message_id.as_str()
    );
    for target in [&local_target, &remote_target] {
        let state: Vec<_> = rows(&records, "client.rpc", "session.state")
            .into_iter()
            .filter(|row| row["attributes"]["target_session_id"] == target.session_id.as_str())
            .collect();
        assert_eq!(state.len(), 1);
        assert_eq!(
            state[0]["attributes"]["target_agent_id"],
            target.agent_id.as_str()
        );
    }
    let rejected_rows: Vec<_> = rows(&records, "host.rpc", "message.send")
        .into_iter()
        .filter(|row| row["message_id"] == rejected.message_id.as_str())
        .collect();
    assert_eq!(rejected_rows.len(), 1);
    let rejected_row = rejected_rows[0];
    assert_eq!(
        rejected_row["in_reply_to"],
        rejected.in_reply_to.unwrap().as_str()
    );
    assert_eq!(rejected_row["attributes"]["rpc_code"], -32602);
    assert_eq!(
        rejected_row["attributes"]["target_agent_id"],
        remote_target.agent_id.as_str()
    );
    assert!(
        rejected_row["attributes"]
            .get("target_session_id")
            .is_none()
    );
    let denied_rows: Vec<_> = rows(&records, "client.rpc", "message.send")
        .into_iter()
        .filter(|row| row["message_id"] == denied.message_id.as_str())
        .collect();
    assert_eq!(denied_rows.len(), 1);
    assert_eq!(denied_rows[0]["fault_type"], refusal.type_name());
    assert_eq!(denied_rows[0]["fault_code"], refusal.code());
    let waits: Vec<_> = rows(&records, "client.rpc", "turn.wait")
        .into_iter()
        .filter(|row| row["attributes"]["target_turn_id"] == turn.as_str())
        .collect();
    assert_eq!(waits.len(), 3);
    assert_eq!(
        waits
            .iter()
            .filter(|row| row["attributes"]["outcome"] == "wait_abandoned"
                && row["attributes"]["delivery"] == "unconfirmed")
            .count(),
        1
    );
    assert_eq!(
        waits
            .iter()
            .filter(|row| row["attributes"]["outcome"] == "wait_timeout"
                && row["attributes"]["delivery"] == "unconfirmed")
            .count(),
        1
    );
    assert_eq!(
        waits
            .iter()
            .filter(|row| row["attributes"]["outcome"] == "response_received")
            .count(),
        1
    );

    // A closed logging outlet is an observed drop, not disk exhaustion. A new
    // actual message remains accepted and executes exactly once afterward.
    let after_close = input();
    let accepted = remote_session
        .send(after_close.clone(), options())
        .await
        .unwrap();
    assert_eq!(accepted.result.delivery, Delivery::Accepted);
    let completed = result(&remote_session, &after_close.message_id).await;
    assert_eq!(
        completed.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(http.count(), 4);
    let logging = observer.info(options()).await.unwrap().logging;
    assert_eq!(logging.status, LoggingState::Degraded);
    assert!(logging.dropped_records.unwrap() > 0);
    assert_eq!(logging.write_failures, Some(0));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    observer.close();
    remote.close();
    server.await.unwrap().unwrap();
    stop(&host).await;
    let requests = http.finish().await;
    assert_eq!(requests.len(), 4);
    assert!(requests[0].to_string().contains(BODY));
    assert!(requests[0].to_string().contains(ROLE));
}
