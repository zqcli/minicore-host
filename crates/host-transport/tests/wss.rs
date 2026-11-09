mod common;
mod wss_common;
use common::*;
use futures_util::SinkExt;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::json;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use wss_common::{FIRST, SECOND, Server};

#[tokio::test]
async fn network_is_opt_in_and_invalid_setup_never_falls_back_to_anonymous() {
    let (host, _) = fixture();
    let (tls, _) = wss_common::certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        matches!(serve_wss(host.clone(), listener, tls).await, Err(CallError::Business(fault)) if fault.kind() == FaultKind::HostAccessInvalid)
    );
    for key in ["", " \t\n"] {
        assert!(host.configure_network_access(Some(key.into())).is_err());
    }
    assert!(host.require_network_access().is_err());
    assert!(host.connect_network(FIRST).is_err());
}

#[tokio::test]
async fn unauthenticated_loopback_cannot_read_or_claim_authority_and_never_occupies_rights() {
    let (host, _) = fixture();
    let server = Server::start(host.clone()).await;
    let mut pending = server.raw().await;
    for method in [
        "host.info",
        "agent.list",
        "session.history.read",
        "events.subscribe",
        "message.send",
    ] {
        let reply = pending
            .request(1, method, json!({"internal":true,"can_operate":true}))
            .await;
        assert_eq!(reply["error"]["code"], -32002);
        assert!(reply.get("result").is_none());
    }
    for params in [
        json!({}),
        json!({"access_key":""}),
        json!({"access_key":"wrong-fixture-secret"}),
        json!({"access_key":FIRST,"internal":true}),
    ] {
        let reply = pending.request(2, "initialize", params).await;
        assert!(reply.get("error").is_some());
        let encoded = reply.to_string();
        assert!(!encoded.contains(FIRST));
        assert!(!encoded.contains("wrong-fixture-secret"));
    }
    // An auth notification cannot initialize or occupy the operator slot.
    pending
        .write(json!({"jsonrpc":"2.0","method":"initialize","params":{"access_key":FIRST}}))
        .await;
    assert_eq!(
        pending.request(3, "host.info", json!({})).await["error"]["code"],
        -32002
    );
    let first = server.client(FIRST).await.unwrap();
    assert!(first.initial_info().can_operate);
    let second = pending.initialize(FIRST).await;
    assert_eq!(second["result"]["can_operate"], false);
    assert_eq!(
        second["result"]["host_id"],
        value(first.initial_info().host_id.clone())
    );
    assert_eq!(
        second["result"]["host_incarnation"],
        value(first.initial_info().host_incarnation.clone())
    );
    assert_eq!(second["result"]["control"]["revision"], 1);
    assert_eq!(pending.initialize(FIRST).await["error"]["code"], -32600);
    assert!(host.info(&host.connect().unwrap().caller()).is_ok());
}

#[tokio::test]
async fn first_initialized_connection_operates_and_observers_do_not_promote() {
    let (host, _) = fixture();
    let server = Server::start(host.clone()).await;
    let first = server.client(FIRST).await.unwrap();
    let second = server.client(FIRST).await.unwrap();
    assert!(!second.initial_info().can_operate);
    assert!(second.info(options(500)).await.is_ok());
    assert!(
        matches!(second.create_agent(create_request(), options(500)).await, Err(CallError::Business(fault)) if fault.kind() == FaultKind::HostAccessReadOnly)
    );
    first.close();
    tokio::time::timeout(Duration::from_secs(3), async {
        while second.info(options(500)).await.unwrap().control.occupied {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(second.create_agent(create_request(), options(500)).await, Err(CallError::Business(fault)) if fault.kind() == FaultKind::HostAccessReadOnly)
    );
    let replacement = server.client(FIRST).await.unwrap();
    assert!(replacement.initial_info().can_operate);
    assert_eq!(replacement.initial_info().control.revision, 3);
    first.close(); // Delayed old cleanup cannot revoke a replacement.
    assert!(create(&replacement).await.info(options(500)).await.is_ok());
    assert_eq!(second.info(options(500)).await.unwrap().control.revision, 3);
}

#[tokio::test]
async fn simultaneous_valid_initializations_grant_exactly_one_operation_right() {
    let (host, _) = fixture();
    let server = Server::start(host).await;
    let (first, second) = tokio::join!(server.client(FIRST), server.client(FIRST));
    let first = first.unwrap();
    let second = second.unwrap();
    assert_ne!(
        first.initial_info().can_operate,
        second.initial_info().can_operate
    );
    assert_eq!(first.initial_info().control.revision, 1);
    assert_eq!(second.initial_info().control.revision, 1);
}

#[tokio::test]
async fn key_publication_closes_network_peers_but_preserves_local_stdio_operator() {
    let (host, _) = fixture();
    let (local, local_server) = stdio(host.clone()).await;
    let trusted = host.connect().unwrap();
    let server = Server::start(host.clone()).await;
    let mut old = server.raw().await;
    assert_eq!(old.initialize(FIRST).await["result"]["can_operate"], false);
    // Trusted setup is not a JSON method. A real reload implementation must use
    // the caller-checked publication hook; here we test embedding publication.
    host.configure_network_access(Some(SECOND.into())).unwrap();
    old.assert_disconnected().await;
    assert!(
        matches!(server.client(FIRST).await, Err(CallError::Business(fault)) if fault.kind() == FaultKind::HostAccessInvalid)
    );
    let new = server.client(SECOND).await.unwrap();
    assert!(!new.initial_info().can_operate);
    assert_eq!(local.info(options(500)).await.unwrap().control.revision, 1);
    assert!(host.info(&trusted.caller()).is_ok());
    create(&local).await;
    local.close();
    local_server.await.unwrap().unwrap();
}

#[tokio::test]
async fn failed_or_unchanged_publication_keeps_network_connection_and_valid_key() {
    let (host, _) = fixture();
    let trusted = host.connect().unwrap();
    let server = Server::start(host.clone()).await;
    let client = server.client(FIRST).await.unwrap();
    let before = client.info(options(500)).await.unwrap().control;
    // Failure represents the configuration owner's version/whole-candidate
    // rejection, not an implementation of the deferred host.reload workflow.
    let rejected = Fault::new(
        FaultKind::HostMethodUnavailable,
        "fixture publication rejected",
    );
    assert_eq!(
        host.publish_network_configuration(
            &trusted.caller(),
            Some(SECOND.into()),
            || Err::<(), _>(rejected.clone())
        ),
        Err(rejected)
    );
    assert!(matches!(
        server.client(SECOND).await,
        Err(CallError::Business(_))
    ));
    host.publish_network_configuration(&trusted.caller(), Some(FIRST.into()), || Ok(()))
        .unwrap();
    assert_eq!(client.info(options(500)).await.unwrap().control, before);
    assert!(server.client(FIRST).await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn old_network_request_blocked_before_acceptance_cannot_borrow_new_key_authority() {
    let (host, resources) = fixture();
    let server = Server::start(host.clone()).await;
    let gate = ValidationGate::new();
    *resources.gate.lock().unwrap() = Some((1, gate.clone()));
    let mut old = server.raw().await;
    old.initialize(FIRST).await;
    old.write(json!({"jsonrpc":"2.0","id":1,"method":"agent.create","params":create_request()}))
        .await;
    gate.entered.acquire().await.unwrap().forget();
    host.configure_network_access(Some(SECOND.into())).unwrap();
    old.assert_disconnected().await;
    let replacement = server.client(SECOND).await.unwrap();
    assert!(replacement.initial_info().can_operate);
    gate.release();
    let local = host.connect().unwrap();
    create(&replacement).await;
    assert_eq!(replacement.agents(options(500)).await.unwrap().len(), 1);
    assert_eq!(host.agents(&local.caller()).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_creation_survives_timeout_disconnect_and_key_rotation() {
    for rotate in [false, true] {
        let (host, resources) = fixture();
        let server = Server::start(host.clone()).await;
        let gate = ValidationGate::new();
        *resources.gate.lock().unwrap() = Some((2, gate.clone()));
        let client = server.client(FIRST).await.unwrap();
        let observer = host.connect().unwrap();
        let mut request = create_request();
        request.initial_message = Some(input("accepted WSS creation input"));
        let operation = request.operation_id.clone();
        let calling = {
            let client = client.clone();
            tokio::spawn(async move { client.create_agent(request, options(40)).await })
        };
        gate.entered.acquire().await.unwrap().forget();
        assert!(client.info(options(500)).await.is_ok());
        assert_eq!(
            calling.await.unwrap().unwrap_err(),
            CallError::WaitTimeout {
                wait_timeout_ms: 40
            }
        );
        let replacement = if rotate {
            host.configure_network_access(Some(SECOND.into())).unwrap();
            server.client(SECOND).await.unwrap()
        } else {
            client.close();
            tokio::time::timeout(Duration::from_secs(3), async {
                while host.info(&observer.caller()).unwrap().control.occupied {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            server.client(FIRST).await.unwrap()
        };
        assert!(replacement.initial_info().can_operate);
        gate.release();
        let created = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let Receipt::AgentCreate(receipt) = replacement
                    .receipt(operation.clone(), options(500))
                    .await
                    .unwrap()
                else {
                    panic!("wrong receipt");
                };
                if receipt.result.steps.message == Some(CreateStep::Succeeded) {
                    break receipt;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(replacement.agents(options(500)).await.unwrap().len(), 1);
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
                .info
                .outcome,
            Some(TurnOutcome::Completed)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wss_receipts_results_and_business_faults_match_typed_and_stdio_reads() {
    let (host, resources) = fixture();
    let server = Server::start(host.clone()).await;
    let client = server.client(FIRST).await.unwrap();
    let typed = HostClient::in_process(host.clone()).unwrap();
    let (stdio, stdio_server) = stdio(host).await;
    let mut request = create_request();
    let mut overrides = SessionOverrides::default();
    overrides.0.model = Some("missing/model".parse().unwrap());
    request.default_session = Some(SessionOptions {
        mode: None,
        overrides: Some(overrides),
    });
    let operation = request.operation_id.clone();
    let partial = client.create_agent(request, options(2_000)).await.unwrap();
    assert!(partial.agent_id.is_some());
    assert!(partial.session_id.is_none());
    assert_eq!(
        value(
            typed
                .receipt(operation.clone(), options(500))
                .await
                .unwrap()
        ),
        value(partial.clone())
    );
    assert_eq!(
        value(stdio.receipt(operation, options(500)).await.unwrap()),
        value(partial)
    );
    let session = create(&client).await;
    session.pause(control(), options(500)).await.unwrap();
    let accepted = session
        .send(input("same owner receipt"), options(500))
        .await
        .unwrap();
    for other in [&typed, &stdio] {
        let other_session = other.session(session.target().clone());
        assert_eq!(
            value(other_session.state(options(500)).await.unwrap()),
            value(session.state(options(500)).await.unwrap())
        );
        assert_eq!(
            other_session
                .message(accepted.message_id.clone(), options(500))
                .await
                .unwrap()
                .input
                .text,
            "same owner receipt"
        );
        assert!(
            matches!(other_session.pause(control(), options(500)).await, Err(CallError::Business(fault)) if fault.kind() == FaultKind::HostAccessReadOnly)
        );
    }
    resources.resolve_failure.store(true, Ordering::SeqCst);
    session.resume(control(), options(500)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while session
            .state(options(500))
            .await
            .unwrap()
            .data
            .block_reason
            .is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let accepted = session
        .send(input("accepted with start error"), options(500))
        .await
        .unwrap();
    assert_eq!(accepted.result.delivery, Delivery::Accepted);
    assert!(accepted.result.start_error.is_some());
    stdio.close();
    stdio_server.await.unwrap().unwrap();
}

#[tokio::test]
async fn websocket_frames_larger_than_the_library_default_are_not_rejected() {
    let (host, _) = fixture();
    let server = Server::start(host).await;
    let mut peer = server.raw().await;
    peer.initialize(FIRST).await;
    // Keep the business request tiny so this test measures framing, not the
    // independently pinned Runtime version's input representation.
    let id = "x".repeat((16 << 20) + 1);
    peer.write(json!({"jsonrpc":"2.0","id":id,"method":"host.info","params":{}}))
        .await;
    let response = peer.read().await;
    assert_eq!(response["id"].as_str().unwrap(), id);
    assert_eq!(response["result"]["lifecycle"], "running");
}

#[tokio::test]
async fn tls_verification_plaintext_rejection_and_protocol_errors_do_not_disable_listener() {
    let (host, _) = fixture();
    let server = Server::start(host).await;
    let untrusted = Arc::new(
        tls::ClientConfig::builder()
            .with_root_certificates(tls::RootCertStore::empty())
            .with_no_client_auth(),
    );
    assert!(matches!(
        HostClient::wss(&server.url(), untrusted, FIRST, options(2_000)).await,
        Err(CallError::Connection { .. })
    ));
    let plaintext = server.url().replacen("wss://", "ws://", 1);
    assert!(matches!(
        HostClient::wss(&plaintext, server.client_tls.clone(), FIRST, options(500)).await,
        Err(CallError::Protocol { .. })
    ));
    let mut bad = server.raw().await;
    bad.socket
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    bad.assert_disconnected().await;
    let mut malformed = server.raw().await;
    malformed
        .socket
        .send(Message::Text("{not json".into()))
        .await
        .unwrap();
    assert_eq!(malformed.read().await["error"]["code"], -32700);
    assert!(
        server
            .client(FIRST)
            .await
            .unwrap()
            .initial_info()
            .can_operate
    );
}

#[tokio::test]
async fn disabling_network_revokes_current_network_and_cannot_admit_anonymous_peers() {
    let (host, _) = fixture();
    let server = Server::start(host.clone()).await;
    let mut old = server.raw().await;
    old.initialize(FIRST).await;
    host.configure_network_access(None).unwrap();
    old.assert_disconnected().await;
    assert!(server.client(FIRST).await.is_err());
    assert!(server.client("").await.is_err());
    let local = HostClient::in_process(host).unwrap();
    assert!(local.initial_info().can_operate);
}
