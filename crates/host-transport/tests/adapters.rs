mod common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::json;
use std::{sync::atomic::Ordering, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn typed_service_runtime_and_stdio_query_parity() {
    let (host, resources) = fixture();
    let handle = host.connect().unwrap();
    let receipt = host
        .create_agent_async(&handle.caller(), create_request())
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    let service = HostService::new(host.clone());
    let accepted = MessageService::send(
        &service,
        handle.caller(),
        SendMessage::new(input("run through the actual Runtime"), target.clone()),
    )
    .await
    .unwrap();
    assert_eq!(accepted.receiver(), target);
    resources.model.started.acquire().await.unwrap().forget();
    let local = HostClient::in_process(host.clone()).unwrap();
    let (remote, server) = stdio(host.clone()).await;
    let typed_session = local.session(target.clone());
    let rpc_session = remote.session(target);
    let turn = running(&typed_session).await;
    resources.model.finish.add_permits(1);
    let result = typed_session
        .wait_turn(turn.clone(), options(2_000))
        .await
        .unwrap();
    assert_eq!(result.data.info.outcome, Some(TurnOutcome::Completed));
    assert!(!result.data.history.is_empty());
    assert_eq!(
        value(local.info(options(2_000)).await.unwrap()),
        value(remote.info(options(2_000)).await.unwrap())
    );
    assert_eq!(
        value(typed_session.state(options(2_000)).await.unwrap()),
        value(rpc_session.state(options(2_000)).await.unwrap())
    );
    assert_eq!(
        value(result),
        value(rpc_session.turn_result(turn, options(2_000)).await.unwrap())
    );
    assert_eq!(
        value(
            typed_session
                .history(
                    HistoryReadRequest {
                        cursor: None,
                        limit: 100
                    },
                    options(2_000)
                )
                .await
                .unwrap()
        ),
        value(
            rpc_session
                .history(
                    HistoryReadRequest {
                        cursor: None,
                        limit: 100
                    },
                    options(2_000)
                )
                .await
                .unwrap()
        )
    );
    assert_eq!(
        value(
            typed_session
                .message(accepted.message_id.clone(), options(2_000))
                .await
                .unwrap()
        ),
        value(
            rpc_session
                .message(accepted.message_id, options(2_000))
                .await
                .unwrap()
        )
    );
    let denied = rpc_session
        .send(input("observer"), options(2_000))
        .await
        .unwrap_err();
    assert!(
        matches!(denied, CallError::Business(f) if f.kind() == Some(FaultKind::HostAccessReadOnly))
    );
    remote.close();
    server.await.unwrap().unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_os_pipes_multiplex_long_wait_cancel_and_queries() {
    let (host, resources) = fixture();
    let (client, server) = stdio_pipes(host.clone()).await;
    let session = create(&client).await;
    let accepted = session
        .send(input("gated Runtime"), options(2_000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&session).await;
    let waiting = {
        let session = session.clone();
        let turn = turn.clone();
        tokio::spawn(async move { session.wait_turn(turn, options(5_000)).await })
    };
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    assert_eq!(
        client.info(options(500)).await.unwrap().lifecycle,
        HostLifecycle::Running
    );
    let paused = session.pause(control(), options(500)).await.unwrap();
    assert_eq!(paused.result.scheduling, Scheduling::Paused);
    assert_eq!(
        session
            .message(accepted.message_id, options(500))
            .await
            .unwrap()
            .data
            .text,
        "gated Runtime"
    );
    let operation_id = OperationId::new().unwrap();
    let cancellation = session
        .cancel_turn(
            TurnCancelRequest {
                operation_id: operation_id.clone(),
                turn_id: turn,
                keep_inbox: true,
            },
            options(500),
        )
        .await
        .unwrap();
    assert!(cancellation.result.cancel_claimed);
    assert!(matches!(
        cancellation.result.turn_state,
        TurnState::Cancelling | TurnState::Finalizing | TurnState::Finished
    ));
    assert_eq!(
        value(client.receipt(operation_id, options(500)).await.unwrap()),
        value(cancellation)
    );
    assert_eq!(
        waiting.await.unwrap().unwrap().data.info.outcome,
        Some(TurnOutcome::Cancelled)
    );
    assert_eq!(resources.model.calls.load(Ordering::SeqCst), 1);
    client.close();
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timeout_and_dropped_wait_do_not_cancel_or_resend_accepted_work() {
    for rpc in [false, true] {
        let (host, resources) = fixture();
        let (client, server) = if rpc {
            let (c, s) = stdio(host).await;
            (c, Some(s))
        } else {
            (HostClient::in_process(host).unwrap(), None)
        };
        let session = create(&client).await;
        session
            .send(input("one input"), options(2_000))
            .await
            .unwrap();
        resources.model.started.acquire().await.unwrap().forget();
        let turn = running(&session).await;
        assert_eq!(
            session
                .wait_turn(turn.clone(), options(5))
                .await
                .unwrap_err(),
            CallError::WaitTimeout { wait_timeout_ms: 5 }
        );
        assert_eq!(
            session
                .turn(turn.clone(), options(500))
                .await
                .unwrap()
                .data
                .state
                .state,
            TurnState::Running
        );
        let waiter = {
            let session = session.clone();
            let turn = turn.clone();
            tokio::spawn(async move { session.wait_turn(turn, options(5_000)).await })
        };
        tokio::task::yield_now().await;
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(
            session
                .turn(turn.clone(), options(500))
                .await
                .unwrap()
                .data
                .state
                .state,
            TurnState::Running
        );
        resources.model.finish.add_permits(1);
        assert_eq!(
            session
                .wait_turn(turn, options(2_000))
                .await
                .unwrap()
                .data
                .info
                .outcome,
            Some(TurnOutcome::Completed)
        );
        assert_eq!(resources.model.calls.load(Ordering::SeqCst), 1);
        assert!(session.pause(control(), options(500)).await.is_ok());
        client.close();
        if let Some(server) = server {
            server.await.unwrap().unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_creation_outlives_timeout_abandonment_and_disconnect() {
    for rpc in [false, true] {
        for abandon in [false, true] {
            let (host, resources) = fixture();
            let gate = ValidationGate::new();
            *resources.gate.lock().unwrap() = Some((2, gate.clone()));
            let (client, server) = if rpc {
                let (c, s) = stdio(host.clone()).await;
                (c, Some(s))
            } else {
                (HostClient::in_process(host.clone()).unwrap(), None)
            };
            let mut request = create_request();
            request.initial_message = Some(input("accepted creation input"));
            let operation = request.operation_id.clone();
            let calling = {
                let client = client.clone();
                tokio::spawn(async move {
                    client
                        .create_agent(request, options(if abandon { 5_000 } else { 30 }))
                        .await
                })
            };
            gate.entered.acquire().await.unwrap().forget();
            // Slow creation cannot obstruct unrelated traffic on the connection.
            assert_eq!(
                client.info(options(500)).await.unwrap().lifecycle,
                HostLifecycle::Running
            );
            if abandon {
                calling.abort();
                let _ = calling.await;
            } else {
                assert_eq!(
                    calling.await.unwrap().unwrap_err(),
                    CallError::WaitTimeout {
                        wait_timeout_ms: 30
                    }
                );
            }
            client.close();
            if let Some(server) = server {
                server.await.unwrap().unwrap();
            }
            let replacement = HostClient::in_process(host.clone()).unwrap();
            assert!(replacement.initial_info().can_operate);
            gate.release();
            let created = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let Receipt::AgentCreate(receipt) = replacement
                        .receipt(operation.clone(), options(500))
                        .await
                        .unwrap()
                    else {
                        panic!("wrong receipt kind")
                    };
                    if receipt.session_id.is_some()
                        && receipt.result.steps.message == Some(CreateStep::Succeeded)
                    {
                        break receipt;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let session = replacement.session(SessionRef {
                agent_id: created.agent_id.unwrap(),
                session_id: created.session_id.unwrap(),
            });
            resources.model.started.acquire().await.unwrap().forget();
            let turn = running(&session).await;
            resources.model.finish.add_permits(1);
            assert_eq!(
                session
                    .wait_turn(turn, options(2_000))
                    .await
                    .unwrap()
                    .data
                    .info
                    .outcome,
                Some(TurnOutcome::Completed)
            );
            assert_eq!(replacement.agents(options(500)).await.unwrap().len(), 1);
            assert_eq!(resources.model.calls.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_initialization_and_json_claims_never_acquire_authority() {
    let (host, _) = fixture();
    let (mut first, first_server) = raw(host.clone());
    assert_eq!(
        first.request(json!(1), "host.info", json!({})).await["error"]["code"],
        -32002
    );
    assert_eq!(
        first
            .request(
                json!(2),
                "initialize",
                json!({"can_operate":true,"internal":true})
            )
            .await["error"]["code"],
        -32602
    );
    first
        .write(json!({"jsonrpc":"2.0","method":"initialize","params":{}}))
        .await;
    assert_eq!(
        first.request(json!(3), "host.info", json!({})).await["error"]["code"],
        -32002
    );
    let direct = host.connect().unwrap();
    assert!(direct.initial_info().can_operate);
    drop(direct);
    assert_eq!(first.initialize().await["result"]["can_operate"], true);
    let response = first
        .request(json!(4), "agent.create", value(create_request()))
        .await;
    let created: AgentCreateReceipt = serde_json::from_value(response["result"].clone()).unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    let (observer, observer_server) = stdio(host.clone()).await;
    assert!(!observer.initial_info().can_operate);
    let request = SendMessage::new(input("forbidden claim"), target.clone());
    let mut claimed = value(request);
    claimed["internal"] = json!(true);
    assert_eq!(
        first.request(json!(5), "message.send", claimed).await["error"]["code"],
        -32602
    );
    assert!(
        matches!(observer.session(target.clone()).send(input("read only"), options(500)).await, Err(CallError::Business(f)) if f.kind() == Some(FaultKind::HostAccessReadOnly))
    );
    drop(first);
    first_server.await.unwrap().unwrap();
    assert!(
        matches!(observer.session(target.clone()).pause(control(), options(500)).await, Err(CallError::Business(f)) if f.kind() == Some(FaultKind::HostAccessReadOnly))
    );
    let (replacement, replacement_server) = stdio(host).await;
    assert!(replacement.initial_info().can_operate);
    // Cleanup of an already-ended connection cannot release this replacement.
    assert!(observer.info(options(500)).await.unwrap().control.occupied);
    assert!(
        replacement
            .session(target)
            .pause(control(), options(500))
            .await
            .is_ok()
    );
    observer.close();
    replacement.close();
    observer_server.await.unwrap().unwrap();
    replacement_server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rpc_protocol_errors_and_notifications_do_not_dispatch_messages() {
    let (host, _) = fixture();
    let (mut peer, server) = raw(host);
    peer.bytes(b"{").await;
    assert_eq!(peer.read().await["error"]["code"], -32700);
    for invalid in [
        json!([]),
        json!({"jsonrpc":"1.0","id":9,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":null,"method":"initialize","params":{}}),
    ] {
        peer.write(invalid).await;
        assert_eq!(peer.read().await["error"]["code"], -32600);
    }
    assert_eq!(peer.initialize().await["result"]["can_operate"], true);
    assert_eq!(
        peer.request(json!(1), "unknown.method", json!({})).await["error"]["code"],
        -32601
    );
    // A newly routed method now validates its actual required parameters.
    assert_eq!(
        peer.request(json!(2), "agent.start", json!({})).await["error"]["code"],
        -32602
    );
    // Every genuinely staged catalog entry still has the explicit business
    // fault. Do not hard-code a placeholder that another topic implements.
    for (index, method) in PUBLIC_METHODS
        .iter()
        .map(|method| method.as_str())
        .filter(|method| !SUPPORTED_METHODS.contains(method))
        .enumerate()
    {
        let response = peer
            .request(json!(format!("staged-{index}")), method, json!({}))
            .await;
        assert_eq!(response["error"]["data"]["code"], 1401, "{method}");
        assert_eq!(
            response["error"]["data"]["type"], "HOST_METHOD_UNAVAILABLE",
            "{method}"
        );
    }
    assert_eq!(
        peer.request(json!(3), "message.send", json!({})).await["error"]["code"],
        -32602
    );
    let response = peer
        .request(json!(4), "agent.create", value(create_request()))
        .await;
    let created: AgentCreateReceipt = serde_json::from_value(response["result"].clone()).unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    peer.request(
        json!(5),
        "session.pause",
        json!({"target":target,"operation_id":OperationId::new().unwrap()}),
    )
    .await;
    let message = input("notification must not be accepted");
    peer.write(json!({"jsonrpc":"2.0","method":"message.send","params":SendMessage::new(message, target.clone())})).await;
    let state = peer
        .request(json!(6), "session.state", json!({"target":target}))
        .await;
    assert_eq!(state["result"]["data"]["next_turn"], json!([]));
    assert_eq!(state["id"], 6);
    drop(peer);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_create_and_accepted_start_error_remain_full_rpc_results() {
    let (host, resources) = fixture();
    let (mut peer, server) = raw(host.clone());
    peer.initialize().await;
    let mut request = create_request();
    let mut overrides = SessionOverrides::default();
    overrides.0.model = Some("missing/model".parse().unwrap());
    request.default_session = Some(SessionOptions {
        mode: None,
        overrides: Some(overrides),
    });
    request.initial_message = Some(input("not attempted"));
    let operation = request.operation_id.clone();
    let response = peer
        .request(json!(123), "agent.create", value(request))
        .await;
    assert!(response.get("error").is_none());
    let partial: AgentCreateReceipt = serde_json::from_value(response["result"].clone()).unwrap();
    assert!(partial.agent_id.is_some());
    assert!(partial.session_id.is_none());
    assert_eq!(partial.result.steps.agent, CreateStep::Succeeded);
    assert!(matches!(
        partial.result.steps.session,
        CreateStep::Failed { .. }
    ));
    assert_eq!(partial.result.steps.message, Some(CreateStep::NotAttempted));
    assert_eq!(
        peer.request(
            json!(124),
            "host.receipt.get",
            json!({"operation_id":operation})
        )
        .await["result"],
        response["result"]
    );
    let response = peer
        .request(json!(125), "agent.create", value(create_request()))
        .await;
    let created: AgentCreateReceipt = serde_json::from_value(response["result"].clone()).unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    resources.resolve_failure.store(true, Ordering::SeqCst);
    peer.request(
        json!(126),
        "message.send",
        value(SendMessage::new(input("first accepted"), target.clone())),
    )
    .await;
    let blocked = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let response = peer
                .request(json!(127), "session.state", json!({"target":target}))
                .await;
            let snapshot: SessionSnapshot =
                serde_json::from_value(response["result"].clone()).unwrap();
            if snapshot.data.block_reason.is_some() {
                break snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let message = input("accepted with known startup failure");
    let response = peer
        .request(
            json!(999),
            "message.send",
            value(SendMessage::new(message.clone(), target)),
        )
        .await;
    assert_eq!(response["id"], 999);
    assert!(response.get("error").is_none());
    let accepted: MessageAccepted = serde_json::from_value(response["result"].clone()).unwrap();
    assert_eq!(accepted.message_id, message.message_id);
    assert_eq!(accepted.result.delivery, Delivery::Accepted);
    assert_eq!(
        accepted.result.start_error,
        blocked.data.block_reason.unwrap().fault
    );
    assert!(accepted.result.start_error.is_some());
    drop(peer);
    server.await.unwrap().unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn os_pipe_frames_have_no_added_application_size_quota_and_shutdown_is_receipt_only() {
    let (host, resources) = fixture();
    let (client, server) = stdio_pipes(host).await;
    let session = create(&client).await;
    session.pause(control(), options(500)).await.unwrap();
    let text = "x\n".repeat(128_000);
    let accepted = session.send(input(&text), options(5_000)).await.unwrap();
    assert_eq!(
        session
            .message(accepted.message_id, options(5_000))
            .await
            .unwrap()
            .data
            .text,
        text
    );
    session.resume(control(), options(500)).await.unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let shutdown = client
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
            options(500),
        )
        .await
        .unwrap();
    assert_eq!(shutdown.result.lifecycle, HostLifecycle::Draining);
    assert_eq!(shutdown.result.strategy, HostShutdownStrategy::Graceful);
    resources.model.finish.add_permits(1);
    client.close();
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn old_channel_request_cannot_borrow_replacement_authority_at_acceptance() {
    let (host, resources) = fixture();
    let gate = ValidationGate::new();
    *resources.gate.lock().unwrap() = Some((1, gate.clone()));
    let (mut original, server) = raw(host.clone());
    original.initialize().await;
    original
        .write(json!({"jsonrpc":"2.0","id":1,"method":"agent.create","params":create_request()}))
        .await;
    gate.entered.acquire().await.unwrap().forget();
    drop(original);
    server.await.unwrap().unwrap();
    let replacement = HostClient::in_process(host).unwrap();
    assert!(replacement.initial_info().can_operate);
    gate.release();
    let replacement_session = create(&replacement).await;
    assert_eq!(replacement.agents(options(500)).await.unwrap().len(), 1);
    assert!(
        replacement_session
            .pause(control(), options(500))
            .await
            .is_ok()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remaining_query_and_canonical_control_routes_return_real_owner_facts() {
    let (host, _) = fixture();
    let (client, server) = stdio(host.clone()).await;
    let observer = host.connect().unwrap();
    let first = create(&client).await;
    let agent = AgentRef {
        host_id: client.initial_info().host_id.clone(),
        agent_id: first.target().agent_id.clone(),
    };
    assert_eq!(client.agents(options(500)).await.unwrap().len(), 1);
    assert_eq!(
        client.agent(agent.clone(), options(500)).await.unwrap().id,
        agent.agent_id
    );
    let created = client
        .create_session(
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
            options(2_000),
        )
        .await
        .unwrap();
    let session = client.session(SessionRef {
        agent_id: created.agent_id.clone(),
        session_id: created.session_id.unwrap(),
    });
    assert_eq!(
        client
            .sessions(agent.clone(), options(500))
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        session.info(options(500)).await.unwrap().id,
        session.target().session_id
    );
    assert!(session.turns(options(500)).await.unwrap().data.is_empty());
    session.pause(control(), options(500)).await.unwrap();
    let loaded = session.load(control(), options(500)).await.unwrap();
    assert_eq!(loaded.kind, SessionControlKind::Load);
    tokio::time::timeout(Duration::from_secs(3), async {
        while session.state(options(500)).await.unwrap().data.residency != Residency::Loaded {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let unloaded = session.unload(control(), options(500)).await.unwrap();
    assert_eq!(unloaded.kind, SessionControlKind::Unload);
    assert_eq!(unloaded.result.scheduling, Scheduling::Paused);
    let cancelled = session
        .cancel(
            SessionCancelRequest {
                operation_id: OperationId::new().unwrap(),
                keep_inbox: false,
            },
            options(500),
        )
        .await
        .unwrap();
    assert!(!cancelled.result.cancel_claimed);
    assert!(cancelled.turn_id.is_none());
    let closed = session
        .close(
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(500),
        )
        .await
        .unwrap();
    assert_eq!(closed.session_id, session.target().session_id);
    assert!(matches!(
        closed.result.lifecycle,
        SessionCloseLifecycle::Closing | SessionCloseLifecycle::Closed
    ));
    let stopped = client
        .stop_agent(
            agent.clone(),
            AgentStopRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(500),
        )
        .await
        .unwrap();
    assert!(matches!(
        stopped.result.lifecycle,
        AgentStopLifecycle::Stopping | AgentStopLifecycle::Stopped
    ));
    assert!(matches!(
        host.agent_info(&observer.caller(), &agent)
            .unwrap()
            .lifecycle,
        AgentLifecycle::Stopping | AgentLifecycle::Stopped
    ));
    client.close();
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receipt_lookup_faults_are_not_method_unavailable() {
    let (host, _) = fixture();
    let (mut peer, server) = raw(host);
    peer.initialize().await;
    let absent = peer
        .request(
            json!(1),
            "host.receipt.get",
            json!({"operation_id":OperationId::new().unwrap()}),
        )
        .await;
    assert_eq!(absent["error"]["data"]["code"], 1202);
    assert_eq!(absent["error"]["data"]["type"], "HOST_RECEIPT_NOT_FOUND");
    let created = peer
        .request(json!(2), "agent.create", value(create_request()))
        .await;
    let created: AgentCreateReceipt = serde_json::from_value(created["result"].clone()).unwrap();
    let params = json!({"target":{"agent_id":created.agent_id,"session_id":created.session_id},"operation_id":OperationId::new().unwrap()});
    for id in [3, 4] {
        assert!(
            peer.request(json!(id), "session.pause", params.clone())
                .await
                .get("result")
                .is_some()
        );
    }
    let ambiguous = peer
        .request(
            json!(5),
            "host.receipt.get",
            json!({"operation_id":params["operation_id"]}),
        )
        .await;
    assert_eq!(ambiguous["error"]["data"]["code"], 1203);
    assert_eq!(ambiguous["error"]["data"]["type"], "HOST_RECEIPT_AMBIGUOUS");
    let invalid_control = peer.request(json!(6), "agent.start", json!({})).await;
    assert_eq!(invalid_control["error"]["code"], -32602);
    assert!(invalid_control["error"]["data"].is_null());
    drop(peer);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_last_client_handle_releases_only_the_connection() {
    let (host, resources) = fixture();
    let (client, server) = stdio(host.clone()).await;
    let session = create(&client).await;
    let target = session.target().clone();
    session
        .send(input("outlive transport"), options(500))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&session).await;
    drop(client);
    let observer = HostClient::in_process(host.clone()).unwrap();
    assert!(!observer.initial_info().can_operate);
    assert_eq!(
        session
            .turn(turn.clone(), options(500))
            .await
            .unwrap()
            .data
            .state
            .state,
        TurnState::Running
    );
    drop(session);
    server.await.unwrap().unwrap();
    let replacement = HostClient::in_process(host).unwrap();
    assert!(replacement.initial_info().can_operate);
    resources.model.finish.add_permits(1);
    assert_eq!(
        replacement
            .session(target)
            .wait_turn(turn, options(2_000))
            .await
            .unwrap()
            .data
            .info
            .outcome,
        Some(TurnOutcome::Completed)
    );
}
