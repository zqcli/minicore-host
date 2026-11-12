use super::support::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use minicore_host_runtime::HistoryItem;
use minicore_host_storage::testing::{read_history_bytes, read_history_text};
use serde_json::{Value, json};

#[tokio::test]
async fn same_text_user_and_peer_keep_actual_source_reply_and_bytes_across_restart() {
    let mut fixture = Fixture::start().await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let initial = config(&fixture.endpoint);
    let b = builder.clone();
    let host = Host::open_configured(root.path(), initial.clone(), move |c| async move {
        Ok(prepared(&b, c).await)
    })
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, "TARGET", true, false).await;
    let parent = create(&host, &caller, "SOURCE", false, true).await;
    let original = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "identical raw text",
        None,
    );
    let first = fixture.next().await;
    let first_http = first.body.clone();
    first.finish(answer("first reply"));
    settled(&host, &caller, &target, 1).await;
    send(
        &host,
        &caller,
        &parent,
        MessageMode::FollowUp,
        "send ordinary peer reply",
        None,
    );
    let command = fixture.next().await;
    let arguments = json!({"action":"send","target":{"kind":"session","agent_id":target.agent_id,"session_id":target.session_id},"text":"identical raw text","in_reply_to":original});
    command.finish(tool_answer("peer-send", "agent", arguments));
    let mut peer_http = None;
    for _ in 0..2 {
        let r = fixture.next().await;
        if r.role() == "TARGET" {
            peer_http = Some(r.body.clone());
            r.finish(answer("peer reply consumed"));
        } else {
            assert_eq!(r.role(), "SOURCE");
            r.finish(answer("peer send accepted"));
        }
    }
    settled(&host, &caller, &target, 2).await;
    settled(&host, &caller, &parent, 1).await;
    let receipt = tool_result(&host, &caller, &parent, "peer-send");
    let peer_id: MessageId = serde_json::from_value(receipt["message_id"].clone()).unwrap();
    let record = host.message(&caller, &target, &peer_id).unwrap();
    assert_eq!(
        record.source,
        MessageSource::Agent {
            source: parent.clone()
        }
    );
    assert_eq!(record.input.in_reply_to, Some(original.clone()));
    let peer_http = peer_http.unwrap();
    assert_fixed(&first_http, "TARGET");
    assert_fixed(&peer_http, "TARGET");
    assert_input(
        &first_http["messages"][2],
        &original,
        MessageSource::User,
        None,
        "identical raw text",
    );
    assert_input(
        &peer_http["messages"][2],
        &original,
        MessageSource::User,
        None,
        "identical raw text",
    );
    assert_input(
        &peer_http["messages"][4],
        &peer_id,
        record.source.clone(),
        Some(&original),
        "identical raw text",
    );
    assert_eq!(peer_http["messages"].as_array().unwrap().len(), 5);
    let native = host.history(&caller, &target).unwrap();
    let native_bytes = serde_json::to_vec(&native).unwrap();
    let readable_before = host
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
    assert!(readable_before.data.next_cursor.is_none());
    for item in &native {
        if let HistoryItem::User(user) = item {
            assert_eq!(user.input.as_text(), "identical raw text");
        }
    }
    let path = history_file(root.path());
    let stored_before = read_history_bytes(&path).unwrap();
    let stored: Vec<Value> = std::str::from_utf8(&stored_before)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let peer_stored = stored
        .iter()
        .find(|r| r["entry"]["data"]["message_id"] == peer_id.as_str())
        .unwrap();
    assert_eq!(peer_stored["entry"]["kind"], "input");
    assert_eq!(
        peer_stored["entry"]["data"]["source"],
        serde_json::to_value(&record.source).unwrap()
    );
    assert_eq!(
        peer_stored["entry"]["data"]["in_reply_to"],
        original.as_str()
    );
    assert_eq!(peer_stored["entry"]["data"]["mode"], "follow_up");
    assert_eq!(
        peer_stored["entry"]["data"]["user"]["input"]["text"],
        "identical raw text"
    );
    stop(&host).await;
    drop(connection);
    drop(host);
    let b = builder.clone();
    let reopened = Host::open_configured(root.path(), initial, move |c| async move {
        Ok(prepared(&b, c).await)
    })
    .await
    .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    // A cold content read preserves the complete saved projection, including
    // provenance, without accepting a new input or loading execution.
    let cold = reopened
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
    assert!(cold.data.next_cursor.is_none());
    assert_eq!(
        serde_json::to_value(&cold.data.items).unwrap(),
        serde_json::to_value(&readable_before.data.items).unwrap()
    );
    assert!(
        !reopened
            .session_context(&caller, &target)
            .await
            .unwrap()
            .data
            .history_loaded
    );
    assert_eq!(read_history_bytes(&path).unwrap(), stored_before);
    let restart_id = send(
        &reopened,
        &caller,
        &target,
        MessageMode::FollowUp,
        "after restart",
        None,
    );
    let restarted = fixture.next().await;
    let restart_http = restarted.body.clone();
    assert_fixed(&restart_http, "TARGET");
    assert_input(
        &restart_http["messages"][4],
        &peer_id,
        record.source.clone(),
        Some(&original),
        "identical raw text",
    );
    assert_input(
        &restart_http["messages"][2],
        &original,
        MessageSource::User,
        None,
        "identical raw text",
    );
    assert_input(
        &restart_http["messages"][6],
        &restart_id,
        MessageSource::User,
        None,
        "after restart",
    );
    assert_eq!(restart_http["messages"].as_array().unwrap().len(), 7);
    let accepted = reopened.history(&caller, &target).unwrap();
    assert_eq!(accepted.len(), native.len() + 1);
    assert_eq!(
        serde_json::to_vec(&accepted[..native.len()]).unwrap(),
        native_bytes
    );
    assert!(
        matches!(&accepted[native.len()..], [HistoryItem::User(user)]
        if user.input.as_text() == "after restart" && user.kind == minicore_host_runtime::UserMessageKind::Prompt)
    );
    let restart_record = reopened.message(&caller, &target, &restart_id).unwrap();
    assert_eq!(restart_record.input.message_id, restart_id);
    assert_eq!(restart_record.source, MessageSource::User);
    assert_eq!(restart_record.input.mode, MessageMode::FollowUp);
    assert!(restart_record.input.in_reply_to.is_none());
    // The fresh bound input is readable, but its response save boundary has
    // not occurred; the entire prior journal remains byte-for-byte intact.
    assert_eq!(read_history_bytes(&path).unwrap(), stored_before);
    restarted.finish(answer("restarted answer"));
    settled(&reopened, &caller, &target, 1).await;
    let final_history = reopened.history(&caller, &target).unwrap();
    assert_eq!(final_history.len(), accepted.len() + 1);
    assert_eq!(&final_history[..accepted.len()], accepted.as_slice());
    let stored_after = read_history_bytes(&path).unwrap();
    assert_eq!(
        &stored_after[..stored_before.len()],
        stored_before.as_slice()
    );
    let appended: Vec<Value> = std::str::from_utf8(&stored_after[stored_before.len()..])
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(appended.len(), 3);
    assert_eq!(appended[0]["entry"]["kind"], "input");
    assert_eq!(
        appended[0]["entry"]["data"]["message_id"],
        restart_id.as_str()
    );
    assert_eq!(
        appended[0]["entry"]["data"]["source"],
        json!({"kind": "user"})
    );
    assert_eq!(appended[0]["entry"]["data"]["mode"], "follow_up");
    assert_eq!(
        appended[0]["entry"]["data"]["user"]["input"]["text"],
        "after restart"
    );
    assert_eq!(appended[1]["entry"]["kind"], "runtime");
    assert_eq!(appended[2]["entry"]["kind"], "turn_finished");
    stop(&reopened).await;
}

#[tokio::test]
async fn same_turn_requests_keep_reply_and_steer_order_without_consuming_follow_up() {
    let mut fixture = Fixture::start().await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let host =
        Host::memory_configured(prepared(&builder, config(&fixture.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, "TARGET", false, false).await;
    let base = send(&host, &caller, &target, MessageMode::FollowUp, "base", None);
    let first = fixture.next().await;
    let first_http = first.body.clone();
    assert_fixed(&first_http, "TARGET");
    assert_input(
        &first_http["messages"][2],
        &base,
        MessageSource::User,
        None,
        "base",
    );
    let follow = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "later turn",
        Some(base.clone()),
    );
    let steer = send(
        &host,
        &caller,
        &target,
        MessageMode::Steer,
        "next step",
        Some(base.clone()),
    );
    first.finish(answer("first internal final"));
    let second = fixture.next().await;
    let second_http = second.body.clone();
    assert_fixed(&second_http, "TARGET");
    assert_input(
        second_http["messages"].as_array().unwrap().last().unwrap(),
        &steer,
        MessageSource::User,
        Some(&base),
        "next step",
    );
    assert!(!second_http.to_string().contains("later turn"));
    assert!(!second_http.to_string().contains(follow.as_str()));
    let base_record = host.message(&caller, &target, &base).unwrap();
    let steer_record = host.message(&caller, &target, &steer).unwrap();
    let bound_turn = |record: &MessageRecord| match &record.binding {
        MessageBinding::Bound { turn_id } => turn_id.clone(),
        other => panic!("not bound: {other:?}"),
    };
    assert_eq!(bound_turn(&base_record), bound_turn(&steer_record));
    second.finish(answer("second internal final"));
    let third = fixture.next().await;
    let third_http = third.body.clone();
    assert_fixed(&third_http, "TARGET");
    assert_input(
        third_http["messages"].as_array().unwrap().last().unwrap(),
        &follow,
        MessageSource::User,
        Some(&base),
        "later turn",
    );
    assert_input(
        &third_http["messages"][4],
        &steer,
        MessageSource::User,
        Some(&base),
        "next step",
    );
    third.finish(answer("later turn final"));
    let turns = settled(&host, &caller, &target, 2).await;
    assert!(
        turns
            .iter()
            .all(|turn| turn.outcome == Some(TurnOutcome::Completed))
    );
    let follow_record = host.message(&caller, &target, &follow).unwrap();
    assert_ne!(bound_turn(&base_record), bound_turn(&follow_record));
    let history = host.history(&caller, &target).unwrap();
    let inputs: Vec<_> = history
        .iter()
        .filter_map(|h| {
            if let HistoryItem::User(u) = h {
                Some(u)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(inputs.len(), 3);
    assert_eq!(inputs[0].loop_id, inputs[1].loop_id);
    assert_ne!(inputs[0].loop_id, inputs[2].loop_id);
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.input.as_text())
            .collect::<Vec<_>>(),
        ["base", "next step", "later turn"]
    );
    assert_eq!(
        inputs.iter().map(|input| input.kind).collect::<Vec<_>>(),
        [
            minicore_host_runtime::UserMessageKind::Prompt,
            minicore_host_runtime::UserMessageKind::Steering,
            minicore_host_runtime::UserMessageKind::Prompt,
        ]
    );
    assert!(matches!(
        &history[..],
        [
            HistoryItem::User(_),
            HistoryItem::Assistant(_),
            HistoryItem::User(_),
            HistoryItem::Assistant(_),
            HistoryItem::User(_),
            HistoryItem::Assistant(_)
        ]
    ));
    let responses = history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::Assistant(response) => Some((response.loop_id, response.request_index)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        responses,
        [
            (inputs[0].loop_id, 0),
            (inputs[1].loop_id, 1),
            (inputs[2].loop_id, 0)
        ]
    );
    assert_eq!(steer_record.input.mode, MessageMode::Steer);
    stop(&host).await;
}

#[tokio::test]
async fn delegated_first_input_has_parent_context_and_existing_completion_stays_exact() {
    let mut fixture = Fixture::start().await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let host =
        Host::memory_configured(prepared(&builder, config(&fixture.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let child = create(&host, &caller, "CHILD", false, false).await;
    let parent = create(&host, &caller, "PARENT", false, true).await;
    send(
        &host,
        &caller,
        &parent,
        MessageMode::FollowUp,
        "delegate and finish first parent turn",
        None,
    );
    let first = fixture.next().await;
    first.finish(tool_answer("delegate-start","agent",json!({"action":"send","delegate":true,"target":{"kind":"session","agent_id":child.agent_id,"session_id":child.session_id},"text":"child raw task"})));
    let mut child_request = None;
    let mut startup_http = None;
    for _ in 0..2 {
        let r = fixture.next().await;
        if r.role() == "CHILD" {
            child_request = Some(r);
        } else {
            assert_eq!(r.role(), "PARENT");
            startup_http = Some(r.body.clone());
            r.finish(answer("parent final before child"));
        }
    }
    settled(&host, &caller, &parent, 1).await;
    let receipt = tool_result(&host, &caller, &parent, "delegate-start");
    let child_request = child_request.unwrap();
    let child_http = child_request.body.clone();
    let task_id: MessageId = serde_json::from_value(receipt["message_id"].clone()).unwrap();
    assert_fixed(&child_http, "CHILD");
    assert_input(
        &child_http["messages"][2],
        &task_id,
        MessageSource::Agent {
            source: parent.clone(),
        },
        None,
        "child raw task",
    );
    let startup = startup_http.unwrap();
    assert_fixed(&startup, "PARENT");
    assert!(startup["messages"].as_array().unwrap().iter().any(|message| message["role"] == "tool" && message["tool_call_id"] == "delegate-start"));
    child_request.finish(answer("actual child result"));
    let notification = fixture.next().await;
    let notification_http = notification.body.clone();
    assert_eq!(notification.role(), "PARENT");
    let (notice_context, notice_text) = input_data(
        notification_http["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
    );
    assert_eq!(
        notice_context["source"],
        json!({"kind": "agent", "source": child})
    );
    let notice: Value = serde_json::from_str(notice_text).unwrap();
    let native_notice = host.history(&caller, &parent).unwrap();
    // This message is still unhanded while its model request is outstanding;
    // its unchanged body is checked after the real response boundary below.
    assert_eq!(notice["source"], serde_json::to_value(&child).unwrap());
    assert_eq!(notice["task_message_id"], receipt["message_id"]);
    assert_eq!(notice["origin"]["tool_call_id"], "delegate-start");
    assert_eq!(notice["status"], "ended");
    assert_eq!(notice["results"][0]["outcome"], "completed");
    assert_eq!(
        notice["results"][0]["output"],
        json!(["actual child result"])
    );
    notification.finish(answer("parent incorporated completion"));
    settled(&host, &caller, &parent, 2).await;
    settled(&host, &caller, &child, 1).await;
    let final_history = host.history(&caller, &parent).unwrap();
    assert!(final_history.iter().any(
        |item| matches!(item, HistoryItem::User(user) if user.input.as_text() == notice_text)
    ));
    assert_eq!(
        &final_history[..native_notice.len()],
        native_notice.as_slice()
    );
    stop(&host).await;
}

#[tokio::test]
async fn stored_partial_is_not_fabricated_into_later_complete_model_context() {
    let mut fixture = Fixture::start().await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let b = builder.clone();
    let host = Host::open_configured(
        root.path(),
        config(&fixture.endpoint),
        move |c| async move { Ok(prepared(&b, c).await) },
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, "TARGET", true, false).await;
    let original = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "before partial",
        None,
    );
    fixture.next().await.finish(format!("data: {}\n\n",json!({"choices":[{"index":0,"delta":{"content":"partial-only-marker"},"finish_reason":null}]})));
    let first = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(first.outcome, Some(TurnOutcome::Failed));
    let result = host.turn_result(&caller, &target, &first.id).unwrap();
    assert_eq!(result.partials.len(), 1);
    let next = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "after partial",
        Some(original.clone()),
    );
    host.resume(&caller, &target).unwrap();
    let request = fixture.next().await;
    let normal_http = request.body.clone();
    assert!(!normal_http.to_string().contains("partial-only-marker"));
    assert!(normal_http.to_string().contains(original.as_str()));
    assert_fixed(&normal_http, "TARGET");
    assert_eq!(normal_http["messages"].as_array().unwrap().len(), 4);
    assert_input(
        &normal_http["messages"][2],
        &original,
        MessageSource::User,
        None,
        "before partial",
    );
    assert_input(
        &normal_http["messages"][3],
        &next,
        MessageSource::User,
        Some(&original),
        "after partial",
    );
    request.finish(answer("after partial reply"));
    settled(&host, &caller, &target, 2).await;
    let stored: Vec<Value> = read_history_text(history_file(root.path()))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(stored.iter().any(|r| r["entry"]["kind"] == "partial"));
    let record = host.message(&caller, &target, &next).unwrap();
    assert_eq!(record.input.in_reply_to, Some(original));
    stop(&host).await;
}
