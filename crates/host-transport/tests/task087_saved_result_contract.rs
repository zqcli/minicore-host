//! Real Host/Runtime, typed operations, OS pipes and authenticated loopback WSS.
//! Only the model event source and test TLS/access credentials are fixtures.
#![cfg(unix)]
mod common;
mod wss_common;
use common::*;
use minicore_host_core::{Host, HostSettings};
use minicore_host_protocol::*;
use minicore_host_transport::*;
use std::{sync::atomic::Ordering, time::Duration};

async fn stopped(client: &HostClient) {
    client
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
            options(3000),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while client.info(options(3000)).await.unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn saved_result_and_wait_share_exact_contract_on_typed_stdio_and_wss() {
    let directory = tempfile::tempdir().unwrap();
    let (_, resources) = fixture();
    let original_host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "saved query adapters".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let original_client = HostClient::in_process(original_host.clone()).unwrap();
    let mut request = create_request();
    request.mode = StorageMode::Persistent;
    request.config.session_defaults.execution.compaction.enabled = false;
    let created = original_client
        .create_agent(request, options(3000))
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    let original = original_client.session(target.clone());
    original
        .send(input("actual saved adapter result"), options(3000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&original).await;
    resources.model.finish.add_permits(1);
    let live = original
        .wait_turn(turn.clone(), options(3000))
        .await
        .unwrap();
    let history = value(&live.data.history);
    let old_identity = live.source;
    assert!(live.data.info.is_some() && live.data.config.is_some());
    assert!(live.data.saved_terminal.is_none());
    stopped(&original_client).await;
    original_client.close();
    drop(original);
    drop(original_client);
    drop(original_host);

    let host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "ignored".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let local = HostClient::in_process(host.clone()).unwrap();
    let (stdio, pipes) = stdio_pipes(host.clone()).await;
    let server = wss_common::Server::start(host.clone()).await;
    let network = server.client(wss_common::FIRST).await.unwrap();
    let path = directory
        .path()
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str())
        .join("history.jsonl");
    let bytes = std::fs::read(&path).unwrap();
    let mut expected = None;
    for loaded in [false, true] {
        if loaded {
            local
                .session(target.clone())
                .load(control(), options(3000))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while local
                    .session(target.clone())
                    .state(options(3000))
                    .await
                    .unwrap()
                    .data
                    .residency
                    != Residency::Loaded
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        let before = value(
            local
                .session(target.clone())
                .state(options(3000))
                .await
                .unwrap(),
        );
        for client in [&local, &stdio, &network] {
            let session = client.session(target.clone());
            for result in [
                session
                    .turn_result(turn.clone(), options(3000))
                    .await
                    .unwrap(),
                session
                    .wait_turn(turn.clone(), options(3000))
                    .await
                    .unwrap(),
            ] {
                assert_eq!(result.source.turn_id, old_identity.turn_id);
                assert_ne!(
                    result.source.session.host_incarnation,
                    old_identity.session.host_incarnation
                );
                assert_eq!(result.source.session.host_id, old_identity.session.host_id);
                assert_eq!(result.source.session.agent_id, target.agent_id);
                assert_eq!(result.source.session.session_id, target.session_id);
                assert!(result.source.load_id.is_none());
                assert!(result.data.info.is_none() && result.data.config.is_none());
                let saved = result.data.saved_terminal.as_ref().unwrap();
                assert_eq!(saved.outcome, TurnOutcome::Completed);
                assert!(saved.reason.is_none());
                assert_eq!(value(&result.data.history), history);
                let wire = value(&result);
                assert!(wire.get("load_id").is_none());
                assert!(wire["data"].get("info").is_none());
                assert!(wire["data"].get("config").is_none());
                if let Some(expected) = &expected {
                    assert_eq!(&wire, expected);
                } else {
                    expected = Some(wire);
                }
            }
            assert!(
                matches!(session.turn_result(TurnId::new().unwrap(), options(3000)).await, Err(CallError::Business(fault)) if fault.kind() == Some(FaultKind::SessionTurnNotFound))
            );
            assert!(session.turns(options(3000)).await.unwrap().data.is_empty());
        }
        assert_eq!(
            value(
                local
                    .session(target.clone())
                    .state(options(3000))
                    .await
                    .unwrap()
            ),
            before
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(resources.model.calls.load(Ordering::SeqCst), 1);
    }
    stdio.close();
    tokio::time::timeout(Duration::from_secs(3), pipes)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    network.close();
    drop(server);
    stopped(&local).await;
    local.close();
}
