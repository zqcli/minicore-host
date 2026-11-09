mod common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;

#[tokio::test]
async fn task071_discovery_pages_and_details_match_direct_and_stdio() {
    let (host, _) = fixture();
    let operator = HostClient::in_process(host.clone()).unwrap();
    let mut target = None;
    for _ in 0..3 {
        let created = operator
            .create_agent(create_request(), options(2000))
            .await
            .unwrap();
        target = Some(AgentRef {
            host_id: created.host_id,
            agent_id: created.agent_id.unwrap(),
        });
    }
    let target = target.unwrap();
    for _ in 0..2 {
        operator
            .create_session(
                SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: target.clone(),
                    mode: None,
                    overrides: None,
                    initial_message: None,
                },
                options(2000),
            )
            .await
            .unwrap();
    }
    let local = HostClient::in_process(host.clone()).unwrap();
    let (remote, server) = stdio(host).await;
    let request = AgentListRequest {
        cursor: None,
        limit: Some(1),
    };
    let first = local
        .list_agents(request.clone(), options(2000))
        .await
        .unwrap();
    assert_eq!(
        value(&first),
        value(remote.list_agents(request, options(2000)).await.unwrap())
    );
    assert_eq!(first.items.len(), 1);
    assert!(first.next_cursor.is_some());
    let rest = remote
        .list_agents(
            AgentListRequest {
                cursor: first.next_cursor,
                limit: None,
            },
            options(2000),
        )
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 2);
    assert!(rest.next_cursor.is_none());
    let request = SessionListRequest {
        cursor: None,
        limit: Some(2),
        mode: Some(StorageMode::Memory),
        lifecycle: Some(SessionLifecycle::Open),
    };
    let first = local
        .list_sessions(target.clone(), request.clone(), options(2000))
        .await
        .unwrap();
    assert_eq!(
        value(&first),
        value(
            remote
                .list_sessions(target.clone(), request, options(2000))
                .await
                .unwrap()
        )
    );
    assert_eq!(first.items.len(), 2);
    assert!(first.next_cursor.is_some());
    let details = remote.agent(target.clone(), options(2000)).await.unwrap();
    assert_eq!(details.sessions.len(), 3);
    assert_eq!(
        value(details),
        value(local.agent(target, options(2000)).await.unwrap())
    );
    for item in first.items {
        assert!(value(item).get("config").is_none());
    }
    for client in [&local, &remote] {
        let error = client
            .list_agents(
                AgentListRequest {
                    limit: Some(0),
                    cursor: None,
                },
                options(2000),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, CallError::Business(fault) if fault.kind() == Some(FaultKind::HostObservationInvalid))
        );
    }
    remote.close();
    server.await.unwrap().unwrap();
    local.close();
    operator.close();
}

#[test]
fn task071_discovery_requests_reject_unknown_or_malformed_fields() {
    assert!(
        serde_json::from_value::<AgentListRequest>(
            serde_json::json!({"cursor":null,"limit":null,"quota":5})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<SessionListRequest>(serde_json::json!({"lifecycle":"retired"}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<SessionListRequest>(
            serde_json::json!({"cursor":"not-a-session-id"})
        )
        .is_err()
    );
    let default: AgentListRequest = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(default, AgentListRequest::default());
}
