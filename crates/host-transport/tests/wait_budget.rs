mod common;
use common::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};

// A deliberately delayed transport peer; this is not a production Host server.
// It forwards selected calls into the actual Host and controls response bytes to
// isolate client-side clock/framing behavior independently of owner scheduling.
async fn delayed_peer(info: ConnectionInfo) -> (HostClient, RawPeer) {
    let (client, peer) = tokio::io::duplex(256);
    let (input, output) = tokio::io::split(client);
    let (reader, writer) = tokio::io::split(peer);
    let mut peer = RawPeer {
        reader: BufReader::new(reader),
        writer,
    };
    let connecting = tokio::spawn(HostClient::stdio(input, output, options(1_000)));
    let initialize = peer.read().await;
    assert_eq!(initialize["method"], "initialize");
    peer.write(json!({"jsonrpc":"2.0","id":initialize["id"],"result":info}))
        .await;
    (connecting.await.unwrap().unwrap(), peer)
}

#[tokio::test(start_paused = true)]
async fn one_default_budget_includes_partial_response_and_never_resends() {
    let (host, _) = fixture();
    let handle = host.connect().unwrap();
    let created = host
        .create_agent_async(&handle.caller(), create_request())
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    host.session_pause(&handle.caller(), &target, control())
        .unwrap();
    let (client, mut peer) = delayed_peer(handle.initial_info().clone()).await;
    let message = input("only one accepted input");
    let calling = {
        let client = client.clone();
        let target = target.clone();
        let message = message.clone();
        tokio::spawn(async move {
            client
                .session(target)
                .send(message, CallOptions::default())
                .await
        })
    };
    let request = peer.read().await;
    assert_eq!(request["method"], "message.send");
    assert!(request["params"].get("wait_timeout_ms").is_none());
    assert_ne!(request["id"], value(message.message_id.clone()));
    let accepted = host
        .send(
            &handle.caller(),
            serde_json::from_value(request["params"].clone()).unwrap(),
        )
        .unwrap();
    // A syntactically complete JSON body is not a complete framed response.
    let response = json!({"jsonrpc":"2.0","id":request["id"],"result":accepted});
    peer.writer
        .write_all(&serde_json::to_vec(&response).unwrap())
        .await
        .unwrap();
    tokio::time::advance(Duration::from_millis(29_999)).await;
    tokio::task::yield_now().await;
    assert!(!calling.is_finished());
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(
        calling.await.unwrap().unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 30_000
        }
    );
    assert_eq!(
        host.session_state(&handle.caller(), &target)
            .unwrap()
            .data
            .next_turn
            .len(),
        1
    );
    peer.writer.write_all(b"\n").await.unwrap(); // Late response for original ID.
    let query = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(500)).await })
    };
    let request = peer.read().await;
    assert_eq!(request["method"], "host.info"); // No resend after the timeout.
    peer.write(
        json!({"jsonrpc":"2.0","id":request["id"],"result":host.info(&handle.caller()).unwrap()}),
    )
    .await;
    assert!(query.await.unwrap().is_ok());
    assert_eq!(
        host.session_state(&handle.caller(), &target)
            .unwrap()
            .data
            .next_turn
            .len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn positive_override_is_one_budget_and_another_call_keeps_working() {
    let (host, _) = fixture();
    let handle = host.connect().unwrap();
    let (client, mut peer) = delayed_peer(handle.initial_info().clone()).await;
    assert!(matches!(
        client.info(options(0)).await,
        Err(CallError::Protocol { .. })
    ));
    let slow = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(40)).await })
    };
    let first = peer.read().await;
    tokio::time::advance(Duration::from_millis(20)).await;
    let fast = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(100)).await })
    };
    let second = peer.read().await;
    assert_ne!(first["id"], second["id"]);
    peer.write(
        json!({"jsonrpc":"2.0","id":second["id"],"result":host.info(&handle.caller()).unwrap()}),
    )
    .await;
    assert!(fast.await.unwrap().is_ok());
    tokio::time::advance(Duration::from_millis(19)).await;
    tokio::task::yield_now().await;
    assert!(!slow.is_finished());
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(
        slow.await.unwrap().unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 40
        }
    );
    peer.write(
        json!({"jsonrpc":"2.0","id":first["id"],"result":host.info(&handle.caller()).unwrap()}),
    )
    .await;
}

#[tokio::test]
async fn malformed_response_and_disconnect_keep_actual_error_provenance() {
    let (host, _) = fixture();
    let handle = host.connect().unwrap();
    let (client, mut peer) = delayed_peer(handle.initial_info().clone()).await;
    let query = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(500)).await })
    };
    let request = peer.read().await;
    peer.write(json!({"jsonrpc":"2.0","id":request["id"],"result":{},"error":{"code":-32600,"message":"both is invalid"}})).await;
    assert!(matches!(
        query.await.unwrap(),
        Err(CallError::Protocol { .. })
    ));
    let (client, mut peer) = delayed_peer(handle.initial_info().clone()).await;
    let query = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(500)).await })
    };
    let _request: Value = peer.read().await;
    drop(peer);
    assert!(matches!(
        query.await.unwrap(),
        Err(CallError::Connection { .. })
    ));
}

fn unknown_fault() -> Value {
    json!({"type":"HOST_FUTURE_FIXTURE_LEAF","code":1499,"message":"future fixture failure","details":{"new_field":[1,"retained"]}})
}

#[tokio::test]
async fn unrecognized_business_fault_preserves_facts_without_failing_another_call() {
    let (host, _) = fixture();
    let handle = host.connect().unwrap();
    let (client, mut peer) = delayed_peer(handle.initial_info().clone()).await;
    let failing = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(1_000)).await })
    };
    let failed_request = peer.read().await;
    let other = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(1_000)).await })
    };
    let other_request = peer.read().await;
    peer.write(json!({"jsonrpc":"2.0","id":failed_request["id"],"error":{"code":-32000,"message":"Business operation failed","data":unknown_fault()}})).await;
    peer.write(json!({"jsonrpc":"2.0","id":other_request["id"],"result":host.info(&handle.caller()).unwrap()})).await;
    let CallError::Business(fault) = failing.await.unwrap().unwrap_err() else {
        panic!("business provenance was lost");
    };
    assert_eq!(fault.kind(), None);
    assert_eq!(value(fault), unknown_fault());
    assert!(other.await.unwrap().is_ok());
}

#[tokio::test]
async fn unrecognized_start_error_does_not_discard_the_real_accepted_receipt() {
    let (host, _) = fixture();
    let handle = host.connect().unwrap();
    let created = host
        .create_agent_async(&handle.caller(), create_request())
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    host.session_pause(&handle.caller(), &target, control())
        .unwrap();
    let (client, mut peer) = delayed_peer(handle.initial_info().clone()).await;
    let calling = {
        let session = client.session(target.clone());
        tokio::spawn(async move {
            session
                .send(
                    input("real accepted input, future wire fault"),
                    options(1_000),
                )
                .await
        })
    };
    let request = peer.read().await;
    let accepted = host
        .send(
            &handle.caller(),
            serde_json::from_value(request["params"].clone()).unwrap(),
        )
        .unwrap();
    let mut wire = value(accepted.clone());
    // This version-compatibility fixture changes only the returned error leaf;
    // delivery itself was accepted by the real Host, not fabricated by a peer.
    wire["result"]["start_error"] = unknown_fault();
    peer.write(json!({"jsonrpc":"2.0","id":request["id"],"result":wire}))
        .await;
    let received = calling.await.unwrap().unwrap();
    assert_eq!(received.message_id, accepted.message_id);
    assert_eq!(received.result.delivery, Delivery::Accepted);
    assert_eq!(value(received.result.start_error.unwrap()), unknown_fault());
    assert_eq!(
        host.session_state(&handle.caller(), &target)
            .unwrap()
            .data
            .next_turn
            .len(),
        1
    );
}

#[tokio::test]
async fn unknown_partial_create_fault_retains_committed_agent_and_failed_session() {
    let (host, _) = fixture();
    let handle = host.connect().unwrap();
    let (client, mut peer) = delayed_peer(handle.initial_info().clone()).await;
    let mut request = create_request();
    let mut overrides = SessionOverrides::default();
    overrides.0.model = Some("missing/model".parse().unwrap());
    request.default_session = Some(SessionOptions {
        mode: None,
        overrides: Some(overrides),
    });
    let calling = {
        let client = client.clone();
        tokio::spawn(async move { client.create_agent(request, options(1_000)).await })
    };
    let request = peer.read().await;
    let partial = host
        .create_agent_async(
            &handle.caller(),
            serde_json::from_value(request["params"].clone()).unwrap(),
        )
        .await
        .unwrap();
    let mut wire = value(partial.clone());
    wire["result"]["steps"]["session"]["error"] = unknown_fault();
    peer.write(json!({"jsonrpc":"2.0","id":request["id"],"result":wire}))
        .await;
    let received = calling.await.unwrap().unwrap();
    assert_eq!(received.agent_id, partial.agent_id);
    assert!(received.agent_id.is_some());
    assert!(received.session_id.is_none());
    assert_eq!(received.result.steps.agent, CreateStep::Succeeded);
    let CreateStep::Failed { error } = received.result.steps.session else {
        panic!("partial failure was lost");
    };
    assert_eq!(value(error), unknown_fault());
    assert_eq!(
        value(
            host.receipt_get(&handle.caller(), &partial.operation_id)
                .unwrap()
        ),
        value(partial)
    );
}
