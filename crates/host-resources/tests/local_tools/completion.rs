//! Real pipe tails are gated by the child, not by a synthetic output sink.
use super::*;
use std::{io::Write, path::Path, time::Instant};

fn fifo(path: &Path) {
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}
async fn next(events: &mut EventSubscription) -> ObservationEvent {
    loop {
        if let Some(ObservationNotification::Event(push)) =
            tokio::time::timeout(Duration::from_secs(10), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            return push.event;
        }
    }
}
async fn cancel_tail(deadline: bool) {
    let command = "printf '%s' \"$BASHPID\" > pid; printf ready; IFS= read -r _ < gate; printf late-tail; printf late-error >&2; printf written > marker; IFS= read -r _ < never";
    let mut replies = vec![(
        tool_answer("cancel-tail", "bash", json!({"command":command})),
        None,
    )];
    if deadline {
        replies.push((answer("timeout handled"), None));
    }
    let _server = Server::start(replies).await;
    let root = tempfile::tempdir().unwrap();
    fifo(&root.path().join("gate"));
    fifo(&root.path().join("never"));
    let mut gate = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("gate"))
        .unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&_server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut req = request(selection(&["bash"]), workspace(root.path()));
    if deadline {
        req.config.session_defaults.execution.tool_timeout_ms = 700;
    }
    let created = host.create_agent_async(&caller, req).await.unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    let mut events = host
        .events_subscribe(
            &caller,
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: target.agent_id.clone(),
                    session_id: target.session_id.clone(),
                },
                categories: [
                    EventCategory::State,
                    EventCategory::Completion,
                    EventCategory::Output,
                ]
                .into_iter()
                .collect(),
            },
        )
        .unwrap();
    send(
        &host,
        &caller,
        &target,
        "produce and retain real post-cancellation pipe tails",
    );
    let source = loop {
        if let ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) =
            next(&mut events).await
        {
            assert_eq!(data.text, "ready");
            break source;
        }
    };
    let pid = std::fs::read_to_string(root.path().join("pid")).unwrap();
    assert!(Path::new(&format!("/proc/{pid}")).exists());
    gate.write_all(b"release\n").unwrap();
    // Deliberately pause only this single-thread executor until the real child
    // has written both tails into its pipes. No fake sink or worker is used.
    let started = Instant::now();
    while !root.path().join("marker").exists() {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    if deadline {
        std::thread::sleep(Duration::from_millis(900));
    } else {
        host.turn_cancel(
            &caller,
            &target,
            TurnCancelRequest {
                operation_id: OperationId::new().unwrap(),
                turn_id: source.turn.turn_id.clone(),
                keep_inbox: false,
            },
        )
        .unwrap();
    }
    let mut published = Vec::new();
    let mut live_at_completion = Vec::new();
    let mut finished_states = 0;
    let mut last_tool_revision = 0;
    let mut tails_after_finish = Vec::new();
    let mut stdout = String::from("ready");
    let mut stderr = String::new();
    loop {
        match next(&mut events).await {
            ObservationEvent::State(StateEvent::Toolcall {
                source: actual,
                data,
            }) if data.state == ToolCallState::Finished => {
                assert_eq!(actual, source);
                assert!(data.runner_revision > last_tool_revision);
                last_tool_revision = data.runner_revision;
                assert!(
                    !Path::new(&format!("/proc/{pid}")).exists(),
                    "public Finished state requires actual cleanup too"
                );
                finished_states += 1;
            }
            ObservationEvent::State(StateEvent::Toolcall { data, .. }) => {
                last_tool_revision = data.runner_revision;
            }
            ObservationEvent::Activity(ActivityEvent::ToolFinished {
                source: actual,
                data,
            }) => {
                assert_eq!(actual, source);
                eprintln!(
                    "TOOL_FINISHED deadline={deadline} positions={:?} child_alive={}",
                    data.output_positions,
                    Path::new(&format!("/proc/{pid}")).exists()
                );
                live_at_completion.push(Path::new(&format!("/proc/{pid}")).exists());
                published.push(data);
            }
            ObservationEvent::Activity(ActivityEvent::ToolOutput {
                source: actual,
                data,
            }) => {
                assert_eq!(actual, source);
                if !published.is_empty() {
                    tails_after_finish.push(data.clone());
                }
                let text = if data.channel == "stdout" {
                    &mut stdout
                } else {
                    &mut stderr
                };
                assert_eq!(data.start, text.len() as u64);
                text.push_str(&data.text);
                eprintln!(
                    "OUTPUT deadline={deadline} after_finish={} data={data:?}",
                    !published.is_empty()
                );
            }
            ObservationEvent::Activity(ActivityEvent::TurnFinished { .. }) => break,
            _ => {}
        }
    }
    assert_eq!(stdout, "readylate-tail");
    assert_eq!(stderr, "late-error");
    let invocation = ToolCallRef {
        turn_id: source.turn.turn_id.clone(),
        tool_call_id: source.tool_call_id.clone(),
    };
    for (channel, text) in [("stdout", &stdout), ("stderr", &stderr)] {
        let actual = host
            .toolcall_output_read(
                &caller,
                &target,
                &invocation,
                &TextReadRequest {
                    channel: channel.into(),
                    start: 0,
                    end: text.len() as u64,
                    max_bytes: 100,
                },
            )
            .unwrap();
        assert_eq!(&actual.data.text, text);
        assert_eq!(actual.source, source);
    }
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let history = host.history(&caller, &target).unwrap();
    let results: Vec<_> = history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::ToolResult(r) if r.call_id.as_str() == "cancel-tail" => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(published.len(), 1);
    assert_eq!(finished_states, 1);
    assert!(tails_after_finish.is_empty());
    assert_eq!(
        results[0].outcome,
        if deadline {
            ToolResultOutcome::Failed
        } else {
            ToolResultOutcome::Cancelled
        }
    );
    eprintln!(
        "FINAL deadline={deadline} stdout={stdout:?} stderr={stderr:?} tails_after_finish={tails_after_finish:?}"
    );
    assert!(
        live_at_completion.iter().all(|alive| !alive),
        "ToolCall reported Finished before actual owned child was waited: {live_at_completion:?}"
    );
    for (channel, text) in [("stdout", stdout), ("stderr", stderr)] {
        assert_eq!(
            published[0]
                .output_positions
                .iter()
                .find(|p| p.channel == channel)
                .map(|p| p.end),
            Some(text.len() as u64),
            "toolcall.finished must carry actual final end for {channel}; tails after published completion: {tails_after_finish:?}"
        );
    }
}
#[tokio::test]
async fn production_cancel_completion_has_actual_pipe_tail_endpoints() {
    cancel_tail(false).await;
}
#[tokio::test]
async fn production_timeout_completion_has_actual_pipe_tail_endpoints() {
    cancel_tail(true).await;
}

async fn completes_before_model(
    name: &str,
    arguments: serde_json::Value,
    outcome: ToolResultOutcome,
    stdout_end: Option<u64>,
) {
    let gate = Arc::new(Semaphore::new(0));
    let mut server = Server::start(vec![
        (tool_answer("normal-call", name, arguments), None),
        (answer("later model reply"), Some(gate.clone())),
    ])
    .await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, selection(&[name]), workspace(root.path())).await;
    let mut events = subscribe(&host, &caller, &target);
    send(
        &host,
        &caller,
        &target,
        "complete a tool before the later model request",
    );
    server.request().await;
    let request = server.request().await;
    assert!(
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "tool")
    );
    let (source, finished) = loop {
        match next(&mut events).await {
            ObservationEvent::Activity(ActivityEvent::ToolFinished { source, data }) => {
                break (source, data);
            }
            ObservationEvent::Activity(ActivityEvent::TurnFinished { .. }) => {
                panic!("model is still gated")
            }
            _ => {}
        }
    };
    assert_eq!(finished.result.outcome, outcome);
    assert_eq!(
        finished
            .output_positions
            .iter()
            .find(|position| position.channel == "stdout")
            .map(|position| position.end),
        stdout_end
    );
    assert_eq!(
        host.turn_info(&caller, &target, &source.turn.turn_id)
            .unwrap()
            .state,
        TurnState::Running
    );
    let tool = host
        .toolcall_get(&caller, &target, &call(&source.turn.turn_id, "normal-call"))
        .unwrap();
    assert_eq!(tool.data.state.state, ToolCallState::Finished);
    assert_eq!(tool.data.result.unwrap().output, finished.result.output);
    assert_eq!(
        native_result(&host.history(&caller, &target).unwrap(), "normal-call").output,
        finished.result.output
    );
    gate.add_permits(1);
    assert_eq!(
        settled(&host, &caller, &target, 1).await[0].outcome,
        Some(TurnOutcome::Completed)
    );
}

#[tokio::test]
async fn normal_local_completion_is_public_before_the_next_model_finishes() {
    completes_before_model(
        "bash",
        json!({"command":"printf finished"}),
        ToolResultOutcome::Success,
        Some(8),
    )
    .await;
}

#[tokio::test]
async fn invalid_local_arguments_finish_without_waiting_for_turn_end() {
    completes_before_model(
        "read",
        json!({"path":"unused"}),
        ToolResultOutcome::Failed,
        None,
    )
    .await;
}
