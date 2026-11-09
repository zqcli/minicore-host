//! Same direct owners through typed calls, real OS pipes, and authenticated TLS WSS.
mod common;
mod wss_common;
use common::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;

async fn exercise(client: HostClient, reader: HostClient) {
    let first = create(&client).await;
    let agent = AgentRef {
        host_id: client.initial_info().host_id.clone(),
        agent_id: first.target().agent_id.clone(),
    };
    let request = SessionDeleteRequest {
        operation_id: OperationId::new().unwrap(),
        close: true,
    };
    assert!(
        matches!(reader.session(first.target().clone()).delete(request.clone(),options(2000)).await.unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::HostAccessReadOnly))
    );
    assert!(
        matches!(first.delete(SessionDeleteRequest{close:false,..request.clone()},options(2000)).await.unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::SessionDeleteInvalid))
    );
    let second = client
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
    let receipt = first.delete(request, options(2000)).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if first.info(options(2000)).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client
            .receipt(receipt.operation_id.clone(), options(2000))
            .await
            .unwrap(),
        Receipt::SessionDelete(receipt)
    );
    assert!(
        first
            .history(
                HistoryReadRequest {
                    cursor: None,
                    limit: 50
                },
                options(2000)
            )
            .await
            .is_err()
    );
    assert!(
        matches!(client.delete_agent(agent.clone(),AgentDeleteRequest{operation_id:OperationId::new().unwrap(),scope:AgentDeleteScope::Empty{}},options(2000)).await.unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::AgentDeleteInvalid))
    );
    let request = AgentDeleteRequest {
        operation_id: OperationId::new().unwrap(),
        scope: AgentDeleteScope::Sessions {
            session_ids: vec![second.session_id.unwrap()],
            close: true,
        },
    };
    assert!(
        matches!(reader.delete_agent(agent.clone(),request.clone(),options(2000)).await.unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::HostAccessReadOnly))
    );
    let receipt = client
        .delete_agent(agent.clone(), request, options(2000))
        .await
        .unwrap();
    assert_eq!(receipt.result.lifecycle, AgentDeleteLifecycle::Retiring);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if client.agent(agent.clone(), options(2000)).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client
            .receipt(receipt.operation_id.clone(), options(2000))
            .await
            .unwrap(),
        Receipt::AgentDelete(receipt)
    );
    client.close();
    reader.close();
}
#[tokio::test]
async fn typed_delete_uses_real_owners() {
    let (host, _) = fixture();
    let client = HostClient::in_process(host.clone()).unwrap();
    let reader = HostClient::in_process(host).unwrap();
    exercise(client, reader).await;
}
#[tokio::test]
async fn os_pipe_stdio_delete_uses_real_owners() {
    let (host, _) = fixture();
    let (client, first) = stdio_pipes(host.clone()).await;
    let (reader, second) = stdio_pipes(host).await;
    exercise(client, reader).await;
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
}
#[tokio::test]
async fn tls_wss_delete_uses_real_owners() {
    let (host, _) = fixture();
    let server = wss_common::Server::start(host).await;
    let client = server.client(wss_common::FIRST).await.unwrap();
    let reader = server.client(wss_common::FIRST).await.unwrap();
    exercise(client, reader).await;
}

#[tokio::test]
async fn in_flight_creation_and_agent_deletion_share_the_direct_owner_admission() {
    let (host, resources) = fixture();
    let client = HostClient::in_process(host.clone()).unwrap();
    let first = create(&client).await;
    let agent = AgentRef {
        host_id: client.initial_info().host_id.clone(),
        agent_id: first.target().agent_id.clone(),
    };
    let gate = ValidationGate::new();
    *resources.gate.lock().unwrap() = Some((
        resources
            .validate_calls
            .load(std::sync::atomic::Ordering::SeqCst)
            + 1,
        gate.clone(),
    ));
    let creating = {
        let client = client.clone();
        let agent = agent.clone();
        tokio::spawn(async move {
            client
                .create_session(
                    SessionCreate {
                        operation_id: OperationId::new().unwrap(),
                        target: agent,
                        mode: None,
                        overrides: None,
                        initial_message: None,
                    },
                    options(5000),
                )
                .await
        })
    };
    gate.entered.acquire().await.unwrap().forget();
    let request = AgentDeleteRequest {
        operation_id: OperationId::new().unwrap(),
        scope: AgentDeleteScope::Sessions {
            session_ids: vec![first.target().session_id.clone()],
            close: true,
        },
    };
    let result = client
        .delete_agent(agent.clone(), request, options(2000))
        .await;
    gate.release();
    assert!(
        matches!(result.unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::AgentDeleteInvalid))
    );
    let second = creating.await.unwrap().unwrap();
    assert_eq!(
        client
            .sessions(agent.clone(), options(2000))
            .await
            .unwrap()
            .len(),
        2
    );
    let receipt = client
        .delete_agent(
            agent.clone(),
            AgentDeleteRequest {
                operation_id: OperationId::new().unwrap(),
                scope: AgentDeleteScope::Sessions {
                    session_ids: vec![
                        first.target().session_id.clone(),
                        second.session_id.unwrap(),
                    ],
                    close: true,
                },
            },
            options(2000),
        )
        .await
        .unwrap();
    assert!(
        client
            .start_agent(
                agent.clone(),
                AgentStartRequest {
                    operation_id: OperationId::new().unwrap()
                },
                options(2000)
            )
            .await
            .is_err()
    );
    assert!(
        client
            .create_session(
                SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: agent,
                    mode: None,
                    overrides: None,
                    initial_message: None
                },
                options(2000)
            )
            .await
            .is_err()
    );
    assert_eq!(
        client
            .receipt(receipt.operation_id.clone(), options(2000))
            .await
            .unwrap(),
        Receipt::AgentDelete(receipt)
    );
    client.close();
}

#[tokio::test]
async fn prepared_agent_and_session_updates_cannot_publish_after_delete_admission() {
    for update_agent in [false, true] {
        let (host, resources) = fixture();
        let client = HostClient::in_process(host).unwrap();
        let child = create(&client).await;
        let agent = AgentRef {
            host_id: client.initial_info().host_id.clone(),
            agent_id: child.target().agent_id.clone(),
        };
        let gate = ValidationGate::new();
        *resources.gate.lock().unwrap() = Some((
            resources
                .validate_calls
                .load(std::sync::atomic::Ordering::SeqCst)
                + 1,
            gate.clone(),
        ));
        let update = {
            let client = client.clone();
            let child = child.clone();
            let agent = agent.clone();
            tokio::spawn(async move {
                if update_agent {
                    client
                        .update_agent(
                            agent,
                            AgentUpdateRequest {
                                operation_id: OperationId::new().unwrap(),
                                expected_config_revision: 0,
                                changes: AgentUpdateChanges {
                                    name: Some("must not publish".into()),
                                    config: Default::default(),
                                },
                            },
                            options(5000),
                        )
                        .await
                        .map(|_| ())
                } else {
                    child
                        .update(
                            SessionUpdateRequest {
                                operation_id: OperationId::new().unwrap(),
                                expected_config_revision: 0,
                                changes: SessionUpdateChanges {
                                    title: NullablePatch::Set("must not publish".into()),
                                    config: Default::default(),
                                },
                            },
                            options(5000),
                        )
                        .await
                        .map(|_| ())
                }
            })
        };
        gate.entered.acquire().await.unwrap().forget();
        let receipt = client
            .delete_agent(
                agent.clone(),
                AgentDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    scope: AgentDeleteScope::Sessions {
                        session_ids: vec![child.target().session_id.clone()],
                        close: true,
                    },
                },
                options(2000),
            )
            .await
            .unwrap();
        gate.release();
        assert!(update.await.unwrap().is_err());
        assert_eq!(
            client
                .receipt(receipt.operation_id.clone(), options(2000))
                .await
                .unwrap(),
            Receipt::AgentDelete(receipt)
        );
        client.close();
    }
}
