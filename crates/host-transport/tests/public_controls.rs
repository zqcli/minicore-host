//! Real owners and Runtime through typed, OS-pipe stdio and TLS WSS adapters.
mod common;
mod os_pipe;
mod wss_common;
use common::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::json;

fn agent_target(client: &HostClient, session: &SessionClient) -> AgentRef {
    AgentRef {
        host_id: client.initial_info().host_id.clone(),
        agent_id: session.target().agent_id.clone(),
    }
}
fn change_agent(name: &str, revision: u64) -> AgentUpdateRequest {
    AgentUpdateRequest {
        operation_id: OperationId::new().unwrap(),
        expected_config_revision: revision,
        changes: AgentUpdateChanges {
            name: Some(name.into()),
            config: AgentConfigPatch {
                role: Some(Role {
                    name: "new default role".into(),
                    ..Role::default()
                }),
                ..Default::default()
            },
        },
    }
}
fn change_session(title: NullablePatch<String>, revision: u64) -> SessionUpdateRequest {
    SessionUpdateRequest {
        operation_id: OperationId::new().unwrap(),
        expected_config_revision: revision,
        changes: SessionUpdateChanges {
            title,
            config: SessionConfigPatch {
                role: Some(Role {
                    name: "Session-only role".into(),
                    ..Role::default()
                }),
                ..Default::default()
            },
        },
    }
}
async fn exercise(client: HostClient, reader: HostClient) {
    let session = create(&client).await;
    let agent = agent_target(&client, &session);
    session.pause(control(), options(2000)).await.unwrap();
    let original = session.info(options(2000)).await.unwrap();
    let update = change_agent("renamed Agent", 0);
    let receipt = client
        .update_agent(agent.clone(), update, options(2000))
        .await
        .unwrap();
    assert_eq!(
        receipt.result,
        MetadataUpdateResult::Committed { config_revision: 1 }
    );
    assert_eq!(
        client
            .receipt(receipt.operation_id.clone(), options(2000))
            .await
            .unwrap(),
        Receipt::AgentUpdate(receipt)
    );
    let current = session.info(options(2000)).await.unwrap();
    assert_eq!(
        current.config, original.config,
        "Agent defaults must not rewrite the existing default Session"
    );
    assert_eq!(current.agent_config_revision, 0);
    let next = client
        .create_session(
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
            options(2000),
        )
        .await
        .unwrap();
    let next = client.session(SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: next.session_id.unwrap(),
    });
    assert_eq!(
        next.info(options(2000)).await.unwrap().config.role.name,
        "new default role"
    );
    assert_eq!(
        next.info(options(2000))
            .await
            .unwrap()
            .agent_config_revision,
        1
    );
    let update = session
        .update(
            change_session(NullablePatch::Set("new title".into()), 0),
            options(2000),
        )
        .await
        .unwrap();
    assert_eq!(
        update.result,
        MetadataUpdateResult::Committed { config_revision: 1 }
    );
    assert_eq!(
        session.info(options(2000)).await.unwrap().title.as_deref(),
        Some("new title")
    );
    assert_eq!(
        next.info(options(2000)).await.unwrap().config.role.name,
        "new default role"
    );
    assert_eq!(
        client
            .agent(agent.clone(), options(2000))
            .await
            .unwrap()
            .info
            .name,
        "renamed Agent"
    );
    session
        .update(change_session(NullablePatch::Clear, 1), options(2000))
        .await
        .unwrap();
    assert_eq!(session.info(options(2000)).await.unwrap().title, None);
    let err = session
        .update(
            change_session(NullablePatch::Set("stale".into()), 0),
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, CallError::Business(f) if f.kind() == Some(FaultKind::SessionConfigConflict))
    );

    let default = client
        .set_default_session(
            agent.clone(),
            AgentSetDefaultSessionRequest {
                operation_id: OperationId::new().unwrap(),
                session_id: Some(next.target().session_id.clone()),
            },
            options(2000),
        )
        .await
        .unwrap();
    assert_eq!(
        default.result,
        AgentSetDefaultSessionResult::Committed {
            default_session_id: Some(next.target().session_id.clone())
        }
    );
    assert_eq!(
        client
            .receipt(default.operation_id.clone(), options(2000))
            .await
            .unwrap(),
        Receipt::AgentSetDefaultSession(default)
    );
    let other = create(&client).await;
    let wrong_owner = client
        .set_default_session(
            agent.clone(),
            AgentSetDefaultSessionRequest {
                operation_id: OperationId::new().unwrap(),
                session_id: Some(other.target().session_id.clone()),
            },
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(wrong_owner, CallError::Business(f) if f.kind() == Some(FaultKind::AgentSessionNotFound))
    );
    client
        .set_default_session(
            agent.clone(),
            AgentSetDefaultSessionRequest {
                operation_id: OperationId::new().unwrap(),
                session_id: None,
            },
            options(2000),
        )
        .await
        .unwrap();
    let no_default = client
        .send(
            SendMessage::new(input("no guessing"), agent.clone()),
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(no_default, CallError::Business(f) if f.kind() == Some(FaultKind::AgentDefaultSessionUnavailable))
    );
    client
        .set_default_session(
            agent.clone(),
            AgentSetDefaultSessionRequest {
                operation_id: OperationId::new().unwrap(),
                session_id: Some(session.target().session_id.clone()),
            },
            options(2000),
        )
        .await
        .unwrap();

    let mut ids = Vec::new();
    for (mode, text) in [
        (MessageMode::FollowUp, "turn first"),
        (MessageMode::Steer, "step first"),
        (MessageMode::FollowUp, "turn second"),
        (MessageMode::Steer, "step second"),
    ] {
        let mut message = input(text);
        message.mode = mode;
        ids.push(message.message_id.clone());
        session.send(message, options(2000)).await.unwrap();
    }
    let list = session.inputs(options(2000)).await.unwrap();
    assert_eq!(
        list.next_step
            .iter()
            .map(|entry| &entry.input.message_id)
            .collect::<Vec<_>>(),
        vec![&ids[1], &ids[3]]
    );
    assert_eq!(
        list.next_turn
            .iter()
            .map(|entry| &entry.input.message_id)
            .collect::<Vec<_>>(),
        vec![&ids[0], &ids[2]]
    );
    assert!(
        list.next_step
            .iter()
            .chain(&list.next_turn)
            .all(|entry| entry.source == MessageSource::User)
    );
    assert_eq!(
        reader
            .session(session.target().clone())
            .inputs(options(2000))
            .await
            .unwrap(),
        list
    );
    let stale = session
        .remove_input(
            InputRemoveRequest {
                operation_id: OperationId::new().unwrap(),
                message_id: ids[1].clone(),
                expected_controller_revision: Some(list.controller_revision.saturating_sub(1)),
            },
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(stale, CallError::Business(f) if f.kind() == Some(FaultKind::SessionInputInvalid))
    );
    assert_eq!(session.inputs(options(2000)).await.unwrap(), list);
    let duplicate = OperationId::new().unwrap();
    let removed = session
        .remove_input(
            InputRemoveRequest {
                operation_id: duplicate.clone(),
                message_id: ids[1].clone(),
                expected_controller_revision: Some(list.controller_revision),
            },
            options(2000),
        )
        .await
        .unwrap();
    assert_eq!(removed.result.queue, InputQueue::NextStep);
    assert_eq!(removed.result.order, list.next_step[0].order);
    assert_eq!(
        client
            .receipt(duplicate.clone(), options(2000))
            .await
            .unwrap(),
        Receipt::InputRemove(removed.clone())
    );
    let later = input("later unrelated");
    let later_id = later.message_id.clone();
    session.send(later, options(2000)).await.unwrap();
    session
        .remove_input(
            InputRemoveRequest {
                operation_id: duplicate.clone(),
                message_id: ids[2].clone(),
                expected_controller_revision: None,
            },
            options(2000),
        )
        .await
        .unwrap();
    assert!(
        matches!(client.receipt(duplicate, options(2000)).await.unwrap_err(), CallError::Business(f) if f.kind() == Some(FaultKind::HostReceiptAmbiguous))
    );
    let remaining = session.inputs(options(2000)).await.unwrap();
    assert_eq!(
        remaining
            .next_turn
            .iter()
            .map(|entry| &entry.input.message_id)
            .collect::<Vec<_>>(),
        vec![&ids[0], &later_id]
    );
    assert_eq!(remaining.next_step[0].input.message_id, ids[3]);
    let missing = session
        .remove_input(
            InputRemoveRequest {
                operation_id: OperationId::new().unwrap(),
                message_id: ids[1].clone(),
                expected_controller_revision: None,
            },
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(missing, CallError::Business(f) if f.kind() == Some(FaultKind::SessionInputNotFound))
    );

    client
        .stop_agent(
            agent.clone(),
            AgentStopRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(2000),
        )
        .await
        .unwrap();
    let started = client
        .start_agent(
            agent.clone(),
            AgentStartRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(2000),
        )
        .await
        .unwrap();
    assert_eq!(started.result.lifecycle, AgentStartLifecycle::Active);
    assert_eq!(
        client
            .receipt(started.operation_id.clone(), options(2000))
            .await
            .unwrap(),
        Receipt::AgentStart(started)
    );
    let repeated_id = OperationId::new().unwrap();
    for _ in 0..2 {
        client
            .start_agent(
                agent.clone(),
                AgentStartRequest {
                    operation_id: repeated_id.clone(),
                },
                options(2000),
            )
            .await
            .unwrap();
    }
    assert!(
        matches!(client.receipt(repeated_id, options(2000)).await.unwrap_err(), CallError::Business(f) if f.kind() == Some(FaultKind::HostReceiptAmbiguous))
    );
    let state = session.state(options(2000)).await.unwrap();
    assert!(state.data.pause_reasons.contains(&PauseReason::Explicit));
    assert!(state.data.active_turn_id.is_none());
    assert_eq!(
        session.inputs(options(2000)).await.unwrap().next_turn,
        remaining.next_turn
    );
    for denied in [
        reader
            .update_agent(agent.clone(), change_agent("denied", 1), options(2000))
            .await
            .map(|_| ()),
        reader
            .start_agent(
                agent.clone(),
                AgentStartRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(2000),
            )
            .await
            .map(|_| ()),
        reader
            .set_default_session(
                agent,
                AgentSetDefaultSessionRequest {
                    operation_id: OperationId::new().unwrap(),
                    session_id: None,
                },
                options(2000),
            )
            .await
            .map(|_| ()),
        reader
            .session(session.target().clone())
            .update(change_session(NullablePatch::Clear, 2), options(2000))
            .await
            .map(|_| ()),
        reader
            .session(session.target().clone())
            .remove_input(
                InputRemoveRequest {
                    operation_id: OperationId::new().unwrap(),
                    message_id: ids[0].clone(),
                    expected_controller_revision: None,
                },
                options(2000),
            )
            .await
            .map(|_| ()),
    ] {
        assert!(
            matches!(denied.unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::HostAccessReadOnly))
        );
    }
    client.close();
    reader.close();
}
#[tokio::test]
async fn typed_all_six_controls_use_real_owners() {
    let (host, _) = fixture();
    let client = HostClient::in_process(host.clone()).unwrap();
    let reader = HostClient::in_process(host).unwrap();
    exercise(client, reader).await;
}
#[tokio::test]
async fn os_pipe_stdio_all_six_controls_use_same_owners() {
    let (host, _) = fixture();
    let (client, server) = stdio_pipes(host.clone()).await;
    let reader = HostClient::in_process(host).unwrap();
    exercise(client, reader).await;
    server.abort();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_wss_all_six_controls_use_same_owners() {
    let (host, _) = fixture();
    let server = wss_common::Server::start(host.clone()).await;
    let client = server.client(wss_common::FIRST).await.unwrap();
    let reader = HostClient::in_process(host).unwrap();
    exercise(client, reader).await;
}
#[tokio::test]
async fn raw_rpc_metadata_rejects_unknown_fields_and_implicit_default_target() {
    let (host, _) = fixture();
    let (mut peer, server) = raw(host);
    peer.initialize().await;
    let agent = peer
        .request(json!(1), "agent.create", value(create_request()))
        .await;
    let target =
        json!({"host_id":agent["result"]["host_id"],"agent_id":agent["result"]["agent_id"]});
    for params in [
        json!({"target":target,"operation_id":OperationId::new().unwrap()}),
        json!({"target":target,"operation_id":OperationId::new().unwrap(),"session_id":null,"internal":true}),
    ] {
        let response = peer
            .request(json!(2), "agent.set_default_session", params)
            .await;
        assert_eq!(response["error"]["code"], -32602);
    }
    let response=peer.request(json!(3),"agent.update",json!({"target":target,"operation_id":OperationId::new().unwrap(),"expected_config_revision":0,"changes":{"config":{"mode":"memory"}}})).await;
    assert_eq!(response["error"]["code"], -32602);
    server.abort();
}
