use super::*;
use std::{collections::BTreeMap, io::Write, path::Path};

fn fifo(path: &Path) {
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}
async fn tool_delta(events: &mut EventSubscription) -> (ToolCallIdentity, OutputDelta) {
    loop {
        if let ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) =
            next(events).await
        {
            return (source, data);
        }
    }
}

#[tokio::test]
async fn live_process_output_preserves_split_utf8_ranges_source_identity_and_native_result() {
    let mut server = Server::start(vec![
        (tool_answer("split-call", "bash", json!({"command":"printf 'a\\344'; IFS= read -r _ < gate; printf '\\270\\255🙂z'; printf '尾' >&2"})), None),
        (answer("split stream finished"), None),
        (tool_answer("split-call", "bash", json!({"command":"printf later"})), None),
        (answer("next Turn finished"), None),
    ]).await;
    let root = tempfile::tempdir().unwrap();
    fifo(&root.path().join("gate"));
    // Keep both ends open so an early tool failure cannot strand a blocking
    // fixture writer. The child still waits for the explicit line below.
    let mut gate = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("gate"))
        .unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, selection(&["bash"]), workspace(root.path())).await;
    let mut events = subscribe(&host, &caller, &target);
    send(&host, &caller, &target, "emit a gated split codepoint");
    server.request().await;
    let (source, first) = tool_delta(&mut events).await;
    assert_eq!(first.channel, "stdout");
    assert_eq!(first.text, "a");
    assert_eq!((first.start, first.end), (0, 1));
    let invocation = call(&source.turn.turn_id, "split-call");
    assert_eq!(source.tool_call_id, invocation.tool_call_id);
    assert_eq!(source.turn.session.agent_id, target.agent_id);
    assert_eq!(source.turn.session.session_id, target.session_id);
    let info = host.info(&caller).unwrap();
    assert_eq!(source.turn.session.host_id, info.host_id);
    assert_eq!(source.turn.session.host_incarnation, info.host_incarnation);
    let turn = host
        .turn_info(&caller, &target, &invocation.turn_id)
        .unwrap();
    assert_eq!(source.turn.load_id, Some(turn.load_id));
    assert_ne!(turn.state, TurnState::Finished);
    assert_eq!(
        read_output(&host, &caller, &target, &invocation, "stdout", 0, 1)
            .unwrap()
            .data
            .text,
        "a"
    );
    let pending = read_output(&host, &caller, &target, &invocation, "stdout", 0, 2).unwrap();
    assert_eq!(pending.data.text, "a");
    assert_eq!(pending.data.end, 1);
    // The process cannot write the remainder until the first partial byte was
    // consumed and its valid prefix was actually observed by the Session.
    gate.write_all(b"release\n").unwrap();
    let mut text = BTreeMap::from([("stdout".to_owned(), first.text)]);
    let mut positions = None;
    loop {
        match next(&mut events).await {
            ObservationEvent::Activity(ActivityEvent::ToolOutput {
                source: current,
                data,
            }) => {
                assert_eq!(current, source);
                data.validate().unwrap();
                let channel = text.entry(data.channel).or_default();
                assert_eq!(data.start, channel.len() as u64);
                channel.push_str(&data.text);
                assert_eq!(data.end, channel.len() as u64);
            }
            ObservationEvent::Activity(ActivityEvent::ToolFinished {
                source: current,
                data,
            }) => {
                assert_eq!(current, source);
                assert_eq!(data.result.outcome, ToolResultOutcome::Success);
                positions = Some(data.output_positions);
            }
            ObservationEvent::Activity(ActivityEvent::TurnFinished { .. }) => break,
            _ => {}
        }
    }
    assert_eq!(text["stdout"], "a中🙂z");
    assert_eq!(text["stderr"], "尾");
    let positions = positions.expect("actual standard tool completion");
    for (channel, content) in &text {
        assert!(
            positions.iter().any(
                |position| &position.channel == channel && position.end == content.len() as u64
            )
        );
        let read = read_output(
            &host,
            &caller,
            &target,
            &invocation,
            channel,
            0,
            content.len() as u64,
        )
        .unwrap();
        assert_eq!(read.source, source);
        assert_eq!(&read.data.text, content);
    }
    assert_eq!(
        read_output(&host, &caller, &target, &invocation, "stdout", 2, 4)
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionOutputRangeInvalid)
    );
    let history = host.history(&caller, &target).unwrap();
    let native = result_json(&history, "split-call");
    assert_eq!(
        native["stdout"]["output"],
        json!({"encoding":"utf8", "data":"a中🙂z"})
    );
    assert_eq!(
        native["stderr"]["output"],
        json!({"encoding":"utf8", "data":"尾"})
    );
    assert_eq!(native["cleanup_confirmed"], true);
    let catchup = subscribe(&host, &caller, &target);
    for (channel, content) in text {
        assert!(
            catchup
                .result()
                .output_positions
                .iter()
                .any(|position| position.source
                    == OutputSource::Toolcall {
                        source: source.clone()
                    }
                    && position.channel == channel
                    && position.end == content.len() as u64)
        );
    }
    // Provider call IDs are only unique within their original public Turn.
    send(&host, &caller, &target, "same call ID in a new Turn");
    let later = settled(&host, &caller, &target, 2)
        .await
        .into_iter()
        .find(|turn| turn.id != invocation.turn_id)
        .unwrap();
    assert_eq!(later.outcome, Some(TurnOutcome::Completed));
    let later_output = read_output(
        &host,
        &caller,
        &target,
        &call(&later.id, "split-call"),
        "stdout",
        0,
        5,
    )
    .unwrap();
    assert_eq!(later_output.data.text, "later");
    assert_eq!(later_output.source.tool_call_id, source.tool_call_id);
    assert_ne!(later_output.source.turn.turn_id, source.turn.turn_id);
    assert_eq!(
        read_output(&host, &caller, &target, &invocation, "stdout", 0, 9)
            .unwrap()
            .data
            .text,
        "a中🙂z"
    );
    let later_result = host.turn_result(&caller, &target, &later.id).unwrap();
    assert_eq!(
        result_json(&later_result.history, "split-call")["stdout"]["output"],
        json!({"encoding":"utf8", "data":"later"})
    );
}

#[tokio::test]
async fn binary_and_incomplete_eof_keep_raw_native_bytes_without_lossy_text_or_false_ranges() {
    let server = Server::start(vec![
        (
            tool_answer(
                "binary",
                "bash",
                json!({"command":"printf 'prefix\\377later'; printf 'err\\344\\270' >&2"}),
            ),
            None,
        ),
        (tool_answer("empty", "bash", json!({"command":":"})), None),
        (answer("binary result retained"), None),
    ])
    .await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, selection(&["bash"]), workspace(root.path())).await;
    send(&host, &caller, &target, "byte exact command results");
    let turn = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    let history = host.history(&caller, &target).unwrap();
    let command = result_json(&history, "binary");
    assert_eq!(
        command["stdout"]["output"],
        json!({"encoding":"bytes", "data":b"prefix\xfflater".to_vec()})
    );
    assert_eq!(
        command["stderr"]["output"],
        json!({"encoding":"bytes", "data":b"err\xe4\xb8".to_vec()})
    );
    assert_eq!(command["stdout"]["complete"], true);
    assert_eq!(command["stderr"]["complete"], true);
    assert_eq!(command["cleanup_confirmed"], true);
    let invocation = call(&turn.id, "binary");
    for (channel, prefix, raw_len) in [("stdout", "prefix", 12), ("stderr", "err", 5)] {
        assert_eq!(
            read_output(
                &host,
                &caller,
                &target,
                &invocation,
                channel,
                0,
                prefix.len() as u64
            )
            .unwrap()
            .data
            .text,
            prefix
        );
        assert_eq!(
            read_output(&host, &caller, &target, &invocation, channel, 0, raw_len)
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionOutputRangeInvalid)
        );
        assert_eq!(
            read_output(
                &host,
                &caller,
                &target,
                &invocation,
                channel,
                prefix.len() as u64,
                prefix.len() as u64
            )
            .unwrap()
            .data
            .text,
            ""
        );
    }
    for channel in ["stdout", "stderr"] {
        assert_eq!(
            read_output(
                &host,
                &caller,
                &target,
                &call(&turn.id, "empty"),
                channel,
                0,
                0
            )
            .unwrap()
            .data
            .text,
            ""
        );
    }
    let subscription = subscribe(&host, &caller, &target);
    for (channel, end) in [("stdout", 6), ("stderr", 3)] {
        assert!(subscription.result().output_positions.iter().any(|position| {
            matches!(&position.source, OutputSource::Toolcall { source } if source.tool_call_id == invocation.tool_call_id)
                && position.channel == channel && position.end == end
        }));
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancelling_actual_bash_waits_for_process_and_retains_both_output_tails() {
    stopped_process(false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn runtime_tool_deadline_joins_actual_bash_before_turn_finish() {
    stopped_process(true).await;
}

#[cfg(target_os = "linux")]
async fn stopped_process(deadline: bool) {
    let mut replies = vec![(
        tool_answer(
            "stopped-call",
            "bash",
            json!({"command":"printf '%s' \"$BASHPID\" > pid; printf stderr-tail >&2; printf stdout-ready; IFS= read -r _ < never; printf forbidden > after-gate"}),
        ),
        None,
    )];
    if deadline {
        replies.push((answer("tool deadline reported"), None));
    }
    let server = Server::start(replies).await;
    let root = tempfile::tempdir().unwrap();
    fifo(&root.path().join("never"));
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut create_request = request(selection(&["bash"]), workspace(root.path()));
    if deadline {
        create_request
            .config
            .session_defaults
            .execution
            .tool_timeout_ms = 1500;
    }
    let created = host
        .create_agent_async(&caller, create_request)
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    let mut events = subscribe(&host, &caller, &target);
    send(
        &host,
        &caller,
        &target,
        "run a real process until owner control",
    );
    let mut ready = String::new();
    let source = loop {
        let (source, delta) = tool_delta(&mut events).await;
        if delta.channel == "stdout" {
            assert_eq!(delta.start, ready.len() as u64);
            ready.push_str(&delta.text);
            if ready == "stdout-ready" {
                break source;
            }
        }
    };
    let pid = std::fs::read_to_string(root.path().join("pid")).unwrap();
    assert!(
        Path::new(&format!("/proc/{pid}")).exists(),
        "actual child must be alive at the readiness gate"
    );
    if !deadline {
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
    let turn = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(turn.id, source.turn.turn_id);
    assert!(turn.cleanup_faults.is_empty(), "{turn:?}");
    assert_eq!(
        turn.outcome,
        Some(if deadline {
            TurnOutcome::Completed
        } else {
            TurnOutcome::Cancelled
        })
    );
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "finished is after the actual child wait"
    );
    assert!(!root.path().join("after-gate").exists());
    let invocation = call(&turn.id, "stopped-call");
    for (channel, text) in [("stdout", "stdout-ready"), ("stderr", "stderr-tail")] {
        let output = read_output(
            &host,
            &caller,
            &target,
            &invocation,
            channel,
            0,
            text.len() as u64,
        )
        .unwrap();
        assert_eq!(output.data.text, text);
        assert_eq!(output.source, source);
    }
    let history = host.history(&caller, &target).unwrap();
    let native = native_result(&history, "stopped-call");
    assert_eq!(
        native.outcome,
        if deadline {
            ToolResultOutcome::Failed
        } else {
            ToolResultOutcome::Cancelled
        }
    );
    let tool = host.toolcall_get(&caller, &target, &invocation).unwrap();
    assert_eq!(tool.data.state.state, ToolCallState::Finished);
    assert_eq!(tool.data.result.unwrap().output, native.output);
    // Neither an observation nor a caller waiting for the result owns the child.
    drop(events);
}

#[tokio::test]
async fn retained_subscriptions_do_not_keep_closed_memory_session_output_or_edits_alive() {
    let server = Server::start(vec![
        (
            tool_answer(
                "write",
                "write",
                json!({"path":"file", "content":"workspace artifact"}),
            ),
            None,
        ),
        (
            tool_answer(
                "process",
                "bash",
                json!({"command":"printf retained-before-close"}),
            ),
            None,
        ),
        (answer("finished"), None),
    ])
    .await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(
        &host,
        &caller,
        selection(&["write", "bash"]),
        workspace(root.path()),
    )
    .await;
    // Intentionally leave all streamed bodies queued throughout execution/close.
    let events = subscribe(&host, &caller, &target);
    let mut states = host.session_states(&caller, &target).unwrap();
    send(&host, &caller, &target, "finish then close");
    let turn = settled(&host, &caller, &target, 1).await.remove(0);
    let record = changes(&host, &caller, &target, ChangeScope::Session).remove(0);
    let retained_copy = host
        .session_change_record(&caller, &target, &record.change_ref)
        .unwrap();
    assert_eq!(
        read_output(
            &host,
            &caller,
            &target,
            &call(&turn.id, "process"),
            "stdout",
            0,
            21
        )
        .unwrap()
        .data
        .text,
        "retained-before-close"
    );
    host.close_session(&caller, &target).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while states.snapshot().is_ok() {
            if states.changed().await.is_err() {
                break;
            }
        }
    })
    .await
    .expect("actual source reclamation with live observer handles");
    assert!(matches!(
        host.session_info(&caller, &target).unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound | FaultKind::HostAgentNotFound)
    ));
    assert!(
        host.session_change_record(&caller, &target, &record.change_ref)
            .is_err()
    );
    assert!(
        read_output(
            &host,
            &caller,
            &target,
            &call(&turn.id, "process"),
            "stdout",
            0,
            21
        )
        .is_err()
    );
    assert_eq!(
        retained_copy.after,
        FileSnapshot::Content(b"workspace artifact".to_vec())
    );
    assert_eq!(
        std::fs::read(root.path().join("file")).unwrap(),
        b"workspace artifact"
    );
    assert!(
        host.events_unsubscribe(
            &caller,
            &UnsubscribeRequest {
                subscription_id: events.result().subscription_id.clone()
            }
        )
        .unwrap()
        .ended
    );
}

#[tokio::test]
async fn persistent_reopen_keeps_native_history_but_not_old_edit_or_process_observations() {
    let server = Server::start(vec![
        (
            tool_answer(
                "durable-write",
                "write",
                json!({"path":"file", "content":"durable artifact"}),
            ),
            None,
        ),
        (
            tool_answer("durable-bash", "bash", json!({"command":"cat file"})),
            None,
        ),
        (answer("durable native result"), None),
        (
            tool_answer(
                "new-write",
                "write",
                json!({"path":"file", "content":"new incarnation"}),
            ),
            None,
        ),
        (answer("new result"), None),
    ])
    .await;
    let data = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let initial = config(&server.endpoint);
    let host = Host::open_configured(data.path(), initial.clone(), |value| async {
        Ok(prepare(&builder, value).await)
    })
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut create_request = request(selection(&["write", "bash"]), workspace(root.path()));
    create_request.mode = StorageMode::Persistent;
    let created = host
        .create_agent_async(&caller, create_request)
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    send(&host, &caller, &target, "write retained native history");
    let turn = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    assert_eq!(turn.persistence, PersistenceState::Persisted);
    let old_identity = host.info(&caller).unwrap();
    let reference = changes(&host, &caller, &target, ChangeScope::Session)
        .remove(0)
        .change_ref;
    let history = host.history(&caller, &target).unwrap();
    let native = native_result(&history, "durable-bash").output.clone();
    let mut states = host.session_states(&caller, &target).unwrap();
    host.unload(&caller, &target).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while states.snapshot().unwrap().data.residency != Residency::Unloaded {
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(
        host.session_change_record(&caller, &target, &reference)
            .unwrap()
            .after,
        FileSnapshot::Content(b"durable artifact".to_vec())
    );
    assert_eq!(
        read_output(
            &host,
            &caller,
            &target,
            &call(&turn.id, "durable-bash"),
            "stdout",
            0,
            16
        )
        .unwrap()
        .data
        .text,
        "durable artifact"
    );
    let mut shutdown = host
        .events_subscribe(
            &caller,
            SubscribeRequest {
                scope: SubscriptionScope::Host {
                    host_id: old_identity.host_id.clone(),
                },
                categories: [EventCategory::State].into_iter().collect(),
            },
        )
        .unwrap();
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    while host.info(&caller).unwrap().lifecycle != HostLifecycle::Stopped {
        next(&mut shutdown).await;
    }
    let reopened = Host::open_configured(data.path(), initial, |value| async {
        Ok(prepare(&builder, value).await)
    })
    .await
    .unwrap();
    let new_connection = reopened.connect().unwrap();
    let caller = new_connection.caller();
    let new_identity = reopened.info(&caller).unwrap();
    assert_eq!(new_identity.host_id, old_identity.host_id);
    assert_ne!(new_identity.host_incarnation, old_identity.host_incarnation);
    let saved = reopened
        .history_read(
            &caller,
            &target,
            &HistoryReadRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(saved.data.items.iter().any(|item| matches!(item, HistoryItemData::ToolResult { tool_call_id, result, .. } if tool_call_id.0 == "durable-bash" && result.output == native)));
    assert_eq!(
        reopened
            .session_changes(
                &caller,
                &target,
                ChangesListRequest {
                    scope: ChangeScope::Session,
                    cursor: None,
                    limit: 100
                }
            )
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionChangeSourceUnavailable)
    );
    assert_eq!(
        reopened
            .session_change_record(&caller, &target, &reference)
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionChangeNotFound)
    );
    assert!(
        read_output(
            &reopened,
            &caller,
            &target,
            &call(&turn.id, "durable-bash"),
            "stdout",
            0,
            16
        )
        .is_err()
    );
    send(
        &reopened,
        &caller,
        &target,
        "new actual write after restart",
    );
    let current = settled(&reopened, &caller, &target, 1).await.remove(0);
    assert_eq!(current.outcome, Some(TurnOutcome::Completed));
    let records = changes(
        &reopened,
        &caller,
        &target,
        ChangeScope::Turn {
            turn_id: current.id.clone(),
        },
    );
    assert_eq!(records.len(), 1);
    let record = reopened
        .session_change_record(&caller, &target, &records[0].change_ref)
        .unwrap();
    assert_eq!(record.tool_call, call(&current.id, "new-write"));
    assert_eq!(
        record.before,
        FileSnapshot::Content(b"durable artifact".to_vec())
    );
    assert_eq!(
        record.after,
        FileSnapshot::Content(b"new incarnation".to_vec())
    );
    assert!(
        reopened
            .session_change_record(&caller, &target, &reference)
            .is_err(),
        "a reused journal index cannot revive an old Turn reference"
    );
}
