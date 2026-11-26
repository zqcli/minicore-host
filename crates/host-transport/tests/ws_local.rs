//! Plain WebSocket (`serve_ws`) over IPv4 and IPv6 loopback with mandatory
//! `initialize { access_key }` authentication and local operator handoff.
mod common;
mod os_pipe;

use futures_util::{SinkExt, StreamExt};
use minicore_host_core::NetworkStatus;
use minicore_host_protocol::FaultKind;
use minicore_host_transport::{HostClient, serve_ws};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::Message};

const KEY_ONE: &str = "ws-local-fixture-key-one";
const KEY_TWO: &str = "ws-local-fixture-key-two";

async fn send_init(addr: SocketAddr, access_key: &str) -> (WebSocketStream<TcpStream>, Value) {
    let url = format!("ws://{addr}/");
    let stream = TcpStream::connect(addr).await.expect("connect tcp");
    let (mut ws, _) = client_async(url, stream).await.expect("ws handshake");
    let frame = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "access_key": access_key },
    });
    ws.send(Message::Text(frame.to_string().into()))
        .await
        .expect("send initialize");
    let reply = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("initialize timeout")
        .expect("stream item")
        .expect("ws frame");
    let Message::Text(text) = reply else {
        panic!("expected text reply, got {reply:?}");
    };
    let parsed: Value = serde_json::from_str(&text).expect("json reply");
    (ws, parsed)
}

#[tokio::test]
async fn serve_ws_supports_ipv6_auth_handoff_and_revocation() {
    let (host, _) = common::fixture();
    let local = HostClient::in_process(host.clone()).unwrap();
    assert!(local.initial_info().can_operate);

    // Bind IPv6 loopback if supported on this runner, else fall back to IPv4.
    let listener = match TcpListener::bind("[::1]:0").await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind("127.0.0.1:0").await.unwrap(),
    };
    let addr = listener.local_addr().unwrap();

    // Must reject start when network access is unconfigured.
    assert!(
        serve_ws(
            host.clone(),
            TcpListener::bind("127.0.0.1:0").await.unwrap()
        )
        .await
        .is_err()
    );

    host.set_network_access_override(Some(KEY_ONE.into()))
        .unwrap();
    local.release_operator().unwrap();
    let mut status_rx = host.subscribe_network_status();

    let server = tokio::spawn(serve_ws(host.clone(), listener));

    // Wrong key is rejected.
    let (_bad_ws, bad_reply) = send_init(addr, KEY_TWO).await;
    assert_eq!(
        bad_reply["error"]["data"]["type"],
        serde_json::to_value(FaultKind::HostAccessInvalid).unwrap()
    );

    // Correct key succeeds and acquires operator control because local released it.
    let (mut ws, ok_reply) = send_init(addr, KEY_ONE).await;
    assert_eq!(ok_reply["result"]["can_operate"], true);
    tokio::time::timeout(Duration::from_secs(3), async {
        while *status_rx.borrow()
            != (NetworkStatus {
                connections: 1,
                operator_is_network: true,
            })
        {
            status_rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(
        local.network_status(),
        NetworkStatus {
            connections: 1,
            operator_is_network: true,
        }
    );

    // Stopping remote clears override key, kicks client, and lets local reclaim operator.
    host.set_network_access_override(None).unwrap();
    server.abort();
    assert_eq!(
        local.network_status(),
        NetworkStatus {
            connections: 0,
            operator_is_network: false,
        }
    );
    local.reclaim_operator().unwrap();

    // Closed peer observes EOF or error on next read.
    let closed = tokio::time::timeout(Duration::from_secs(3), ws.next()).await;
    assert!(matches!(
        closed,
        Ok(None) | Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_)))
    ));
}
