//! Independent SPEC contract probes. Real Host/Runtime owners; explicit fixture model.
mod common;
#[path = "query_contracts/drop_cleanup.rs"]
mod drop_cleanup;
mod os_pipe;
use common::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_get_must_return_original_bound_config_after_session_update() {
    let (host, resources) = fixture();
    let client = HostClient::in_process(host).unwrap();
    let session = create(&client).await;
    let original = session.info(options(2000)).await.unwrap().config;
    session
        .send(input("hold the first turn"), options(2000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&session).await;
    session
        .update(
            SessionUpdateRequest {
                operation_id: OperationId::new().unwrap(),
                expected_config_revision: 0,
                changes: SessionUpdateChanges {
                    title: NullablePatch::Unchanged,
                    config: SessionConfigPatch {
                        role: Some(Role {
                            name: "future role".into(),
                            ..Role::default()
                        }),
                        ..Default::default()
                    },
                },
            },
            options(2000),
        )
        .await
        .unwrap();
    let snapshot = value(session.turn(turn.clone(), options(2000)).await.unwrap());
    resources.model.finish.add_permits(1);
    let _ = session
        .wait_turn(turn.clone(), options(2000))
        .await
        .unwrap();
    let completed = value(session.turn(turn, options(2000)).await.unwrap());
    println!("turn.get active after next-Turn update: {snapshot}");
    println!("turn.get completed after next-Turn update: {completed}");
    assert_eq!(snapshot["data"]["config_revision"], json!(0));
    assert_eq!(completed["data"]["config_revision"], json!(0));
    let original = value(original);
    assert!(
        snapshot["data"].get("config") == Some(&original)
            && completed["data"].get("config") == Some(&original),
        "SPEC:170 requires the original bound config for both active and completed Turn queries; neither returns it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_result_and_wait_must_carry_current_host_and_original_target() {
    let (host, resources) = fixture();
    let client = HostClient::in_process(host.clone()).unwrap();
    let session = create(&client).await;
    session
        .send(input("produce a completed result"), options(2000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&session).await;
    resources.model.finish.add_permits(1);
    session
        .wait_turn(turn.clone(), options(2000))
        .await
        .unwrap();
    let (mut peer, server) = raw(host);
    peer.initialize().await;
    let target = value(session.target());
    let mut missing = Vec::new();
    for method in ["turn.result", "turn.wait", "turn.list"] {
        let params = if method == "turn.list" {
            json!({"target":target})
        } else {
            json!({"target":target,"turn_id":turn})
        };
        let response = peer.request(json!(method), method, params).await;
        let result = response.get("result").expect("successful real query");
        println!("{method}: {result}");
        for field in [
            "host_id",
            "host_incarnation",
            "agent_id",
            "session_id",
            "data",
        ] {
            if result.get(field).is_none() {
                missing.push(format!("{method}:{field}"));
            }
        }
    }
    server.abort();
    assert!(
        missing.is_empty(),
        "SPEC:1469 current Host/run/target and data absent: {missing:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_get_must_carry_current_host_and_receiver_identity() {
    let (host, _) = fixture();
    let client = HostClient::in_process(host.clone()).unwrap();
    let session = create(&client).await;
    session.pause(control(), options(2000)).await.unwrap();
    let message = input("queued without execution");
    session.send(message.clone(), options(2000)).await.unwrap();
    let (mut peer, server) = raw(host);
    peer.initialize().await;
    let response = peer
        .request(
            json!(1),
            "message.get",
            json!({"target":session.target(),"message_id":message.message_id}),
        )
        .await;
    let result = &response["result"];
    println!("message.get: {result}");
    server.abort();
    let missing: Vec<_> = [
        "host_id",
        "host_incarnation",
        "agent_id",
        "session_id",
        "data",
    ]
    .into_iter()
    .filter(|field| result.get(*field).is_none())
    .collect();
    assert!(
        missing.is_empty(),
        "SPEC:1469 receiver identity and current Host/run absent: {missing:?}"
    );
}

fn assert_no_runtime_keys(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                assert!(
                    !["loop_id", "request_index", "provider_replay"].contains(&key.as_str()),
                    "internal key {key}"
                );
                assert_no_runtime_keys(value);
            }
        }
        serde_json::Value::Array(values) => values.iter().for_each(assert_no_runtime_keys),
        _ => {}
    }
}
fn assert_receiver(value: &serde_json::Value, expected: &SessionIdentity) {
    assert_eq!(value["host_id"], json!(expected.host_id));
    assert_eq!(value["host_incarnation"], json!(expected.host_incarnation));
    assert_eq!(value["agent_id"], json!(expected.agent_id));
    assert_eq!(value["session_id"], json!(expected.session_id));
    assert!(value.get("data").is_some());
    for key in [
        "host_id",
        "host_incarnation",
        "agent_id",
        "session_id",
        "turn_id",
        "load_id",
        "message_id",
    ] {
        assert!(
            value["data"].get(key).is_none(),
            "duplicate outer identity {key}"
        );
    }
    assert_no_runtime_keys(value);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_identity_and_values_match_over_real_pipes_for_readonly_and_replacement_operator() {
    let (host, resources) = fixture();
    let operator = HostClient::in_process(host.clone()).unwrap();
    let session = create(&operator).await;
    let target = session.target().clone();
    let info = operator.info(options(2000)).await.unwrap();
    let expected = SessionIdentity {
        host_id: info.host_id,
        host_incarnation: info.host_incarnation,
        agent_id: target.agent_id.clone(),
        session_id: target.session_id.clone(),
    };
    let (reader, server) = stdio_pipes(host.clone()).await;
    assert!(!reader.initial_info().can_operate);
    let observed = reader.session(target.clone());
    session.pause(control(), options(2000)).await.unwrap();
    let mut message = input("actual retained message");
    message.in_reply_to = Some(MessageId::new().unwrap());
    session.send(message.clone(), options(2000)).await.unwrap();
    let queued = value(
        observed
            .message(message.message_id.clone(), options(2000))
            .await
            .unwrap(),
    );
    assert_receiver(&queued, &expected);
    assert_eq!(queued["message_id"], json!(message.message_id));
    assert_eq!(queued["data"]["in_reply_to"], json!(message.in_reply_to));
    assert_eq!(queued["data"]["binding"]["state"], "queued");
    assert_eq!(
        queued,
        value(
            session
                .message(message.message_id.clone(), options(2000))
                .await
                .unwrap()
        )
    );
    let empty = value(observed.turns(options(2000)).await.unwrap());
    assert_receiver(&empty, &expected);
    assert_eq!(empty["data"], json!([]));
    session.resume(control(), options(2000)).await.unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&session).await;
    let running = value(observed.turn(turn.clone(), options(2000)).await.unwrap());
    assert_receiver(&running, &expected);
    assert_eq!(running["turn_id"], json!(turn));
    let load = running["load_id"].clone();
    assert!(load.is_string());
    let states = reader
        .subscribe(
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: target.agent_id.clone(),
                    session_id: target.session_id.clone(),
                },
                categories: [EventCategory::State].into_iter().collect(),
            },
            options(2000),
        )
        .await
        .unwrap();
    for state in &states.result().initial_states {
        let state = value(state);
        assert!(state["data"].get("config").is_none());
        assert!(state["data"].get("role").is_none());
    }
    reader
        .unsubscribe(
            UnsubscribeRequest {
                subscription_id: states.result().subscription_id.clone(),
            },
            options(2000),
        )
        .await
        .unwrap();
    drop(states);
    assert!(matches!(
        observed
            .wait_turn(turn.clone(), options(5))
            .await
            .unwrap_err(),
        CallError::WaitTimeout { .. }
    ));
    operator.close();
    let replacement = HostClient::in_process(host.clone()).unwrap();
    assert!(replacement.initial_info().can_operate);
    let replacement_session = replacement.session(target.clone());
    resources.model.finish.add_permits(1);
    let waited = value(
        observed
            .wait_turn(turn.clone(), options(2000))
            .await
            .unwrap(),
    );
    let result = value(
        replacement_session
            .turn_result(turn.clone(), options(2000))
            .await
            .unwrap(),
    );
    assert_eq!(waited, result);
    assert_receiver(&result, &expected);
    assert_eq!(result["turn_id"], json!(turn));
    assert_eq!(result["load_id"], load);
    assert_eq!(result["data"]["info"]["outcome"], "completed");
    assert_eq!(result["data"]["info"]["persistence"], "not_applicable");
    assert_eq!(result["data"]["history"][1]["response_index"], 0);
    let listed = value(observed.turns(options(2000)).await.unwrap());
    assert_receiver(&listed, &expected);
    assert_eq!(
        listed,
        value(replacement_session.turns(options(2000)).await.unwrap())
    );
    assert_eq!(listed["data"][0]["turn_id"], json!(turn));
    assert_eq!(listed["data"][0]["load_id"], load);
    let bound = value(
        observed
            .message(message.message_id.clone(), options(2000))
            .await
            .unwrap(),
    );
    assert_receiver(&bound, &expected);
    assert_eq!(bound["data"]["binding"]["turn_id"], json!(turn));
    // Keep the direct Agent alive and verify an existing different Session
    // never inherits the queried Turn or Message from the caller's context.
    let other = replacement
        .create_session(
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: AgentRef {
                    host_id: expected.host_id.clone(),
                    agent_id: expected.agent_id.clone(),
                },
                mode: None,
                overrides: None,
                initial_message: None,
            },
            options(2000),
        )
        .await
        .unwrap();
    let other = reader.session(SessionRef {
        agent_id: expected.agent_id.clone(),
        session_id: other.session_id.unwrap(),
    });
    assert!(
        matches!(other.turn(turn.clone(), options(2000)).await.unwrap_err(), CallError::Business(fault) if fault.kind() == Some(FaultKind::SessionTurnNotFound))
    );
    assert!(
        matches!(other.message(message.message_id.clone(), options(2000)).await.unwrap_err(), CallError::Business(fault) if fault.kind() == Some(FaultKind::SessionInputNotFound))
    );
    replacement_session
        .close(
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(2000),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Err(CallError::Business(fault)) = observed.turns(options(2000)).await {
                assert_eq!(fault.kind(), Some(FaultKind::AgentSessionNotFound));
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for error in [
        observed
            .message(message.message_id, options(2000))
            .await
            .unwrap_err(),
        observed
            .turn(turn.clone(), options(2000))
            .await
            .unwrap_err(),
        observed
            .turn_result(turn.clone(), options(2000))
            .await
            .unwrap_err(),
        observed.wait_turn(turn, options(2000)).await.unwrap_err(),
    ] {
        assert!(
            matches!(error, CallError::Business(fault) if fault.kind() == Some(FaultKind::AgentSessionNotFound))
        );
    }
    reader.close();
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_result_and_wait_keep_complete_and_partial_indexes_within_one_turn() {
    let (host, resources) = fixture();
    let (client, server) = common::pipes::stdio(host.clone(), options(2000)).await;
    let session = create(&client).await;
    let mut outputs = client
        .subscribe(
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: session.target().agent_id.clone(),
                    session_id: session.target().session_id.clone(),
                },
                categories: [EventCategory::Output].into_iter().collect(),
            },
            options(2000),
        )
        .await
        .unwrap();
    session
        .send(input("first complete request"), options(2000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&session).await;
    let identity = session
        .turn(turn.clone(), options(2000))
        .await
        .unwrap()
        .source;
    let mut continuation = input("next request, same Turn");
    continuation.mode = MessageMode::Steer;
    session.send(continuation, options(2000)).await.unwrap();
    resources.model.finish.add_permits(1);
    resources.model.started.acquire().await.unwrap().forget();
    assert_eq!(running(&session).await, turn);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let minicore_host_core::ObservationNotification::Event(push) =
                outputs.recv().await.unwrap().unwrap()
                && let minicore_host_core::ObservationEvent::Activity(
                    minicore_host_core::ActivityEvent::ModelOutput { source, .. },
                ) = push.event
                && source.response_index == 1
            {
                assert_eq!(source.turn, identity);
                break;
            }
        }
    })
    .await
    .unwrap();
    session
        .cancel_turn(
            TurnCancelRequest {
                operation_id: OperationId::new().unwrap(),
                turn_id: turn.clone(),
                keep_inbox: false,
            },
            options(2000),
        )
        .await
        .unwrap();
    let waited = session
        .wait_turn(turn.clone(), options(2000))
        .await
        .unwrap();
    assert_eq!(waited.source, identity);
    assert_eq!(
        waited.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Cancelled)
    );
    assert_eq!(waited.data.partials.len(), 1);
    assert_eq!(waited.data.partials[0].response_index, 1);
    assert_eq!(
        waited
            .data
            .history
            .iter()
            .filter_map(|item| item.response_index)
            .collect::<Vec<_>>(),
        [0]
    );
    let retained_reader = host.connect().unwrap();
    let native = host
        .turn_result(&retained_reader.caller(), session.target(), &turn)
        .unwrap();
    assert_eq!(native.partials[0].partial.request_index, 1);
    let loops: Vec<_> = native
        .history
        .iter()
        .filter_map(|item| {
            if let minicore_host_runtime::HistoryItem::User(user) = item {
                Some(user.loop_id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(loops.len(), 2);
    assert_eq!(loops[0], loops[1]);
    let (mut peer, raw_server) = raw(host);
    peer.initialize().await;
    for method in ["turn.result", "turn.wait"] {
        let response = peer
            .request(
                json!(method),
                method,
                json!({"target":session.target(),"turn_id":turn}),
            )
            .await;
        let result = response.get("result").expect("retained real result");
        assert_eq!(result, &value(&waited));
        assert_no_runtime_keys(result);
    }
    client
        .unsubscribe(
            UnsubscribeRequest {
                subscription_id: outputs.result().subscription_id.clone(),
            },
            options(2000),
        )
        .await
        .unwrap();
    drop(peer);
    raw_server.await.unwrap().unwrap();
    // Request EOF retires the actual server before closing its response reader.
    // Drop the already-ended SDK stream only after connection cleanup, so its
    // best-effort unsubscribe cannot race the reader that would receive it.
    server
        .close(&client, Some(std::time::Duration::from_secs(3)))
        .await;
    drop(outputs);
}
