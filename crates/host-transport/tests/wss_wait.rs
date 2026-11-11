mod common;
mod os_pipe;
mod wss_common;
use common::*;
use futures_util::{SinkExt, StreamExt};
use minicore_host_core::{ConnectionHandle, Host};
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::{
    WebSocketStream, accept_async_with_config,
    tungstenite::{
        Message,
        protocol::{
            WebSocketConfig,
            frame::{
                Frame,
                coding::{Data, OpCode},
            },
        },
    },
};
use wss_common::{FIRST, Server};

// Deliberately delayed TLS/WebSocket test peer. It authenticates and dispatches
// selected reads into the actual Host, then controls only response framing.
// This is a transport-clock fixture, not the production WSS server.
struct DelayedPeer {
    socket: WebSocketStream<tokio_rustls::server::TlsStream<TcpStream>>,
    handle: ConnectionHandle,
}
impl DelayedPeer {
    async fn read(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match self.socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => break serde_json::from_str(&text).unwrap(),
                    Message::Ping(_) | Message::Pong(_) => {}
                    _ => panic!("unexpected fixture frame"),
                }
            }
        })
        .await
        .unwrap()
    }
    async fn response(&mut self, value: Value) {
        self.socket
            .send(Message::Text(serde_json::to_string(&value).unwrap().into()))
            .await
            .unwrap();
    }
}
async fn delayed_peer(host: Arc<Host>) -> (HostClient, DelayedPeer) {
    host.configure_network_access(Some(FIRST.into())).unwrap();
    let (server_tls, client_tls) = wss_common::certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}/", listener.local_addr().unwrap().port());
    let connecting =
        tokio::spawn(async move { HostClient::wss(&url, client_tls, FIRST, options(2_000)).await });
    let (tcp, _) = listener.accept().await.unwrap();
    let tls = TlsAcceptor::from(server_tls).accept(tcp).await.unwrap();
    let config = WebSocketConfig::default()
        .max_frame_size(None)
        .max_message_size(None)
        .max_write_buffer_size(usize::MAX);
    let socket = accept_async_with_config(tls, Some(config)).await.unwrap();
    // Handle creation follows actual authentication, not connection arrival.
    let mut socket = socket;
    let Message::Text(auth) = socket.next().await.unwrap().unwrap() else {
        panic!("no initialization");
    };
    let auth: Value = serde_json::from_str(&auth).unwrap();
    assert_eq!(auth["method"], "initialize");
    let handle = host
        .connect_network(auth["params"]["access_key"].as_str().unwrap())
        .unwrap();
    let mut peer = DelayedPeer { socket, handle };
    peer.response(json!({"jsonrpc":"2.0","id":auth["id"],"result":peer.handle.initial_info()}))
        .await;
    (connecting.await.unwrap().unwrap(), peer)
}

#[tokio::test]
async fn complete_json_in_unfinished_websocket_message_does_not_reset_the_wait_budget() {
    let (host, _) = fixture();
    let (client, mut peer) = delayed_peer(host.clone()).await;
    let call = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(80)).await })
    };
    let request = peer.read().await;
    assert_eq!(request["method"], "host.info");
    assert!(request["params"].get("wait_timeout_ms").is_none());
    let response = json!({"jsonrpc":"2.0","id":request["id"],"result":host.info(&peer.handle.caller()).unwrap()});
    let fragment = Frame::message(
        serde_json::to_vec(&response).unwrap(),
        OpCode::Data(Data::Text),
        false,
    );
    peer.socket.send(Message::Frame(fragment)).await.unwrap();
    assert_eq!(
        call.await.unwrap().unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 80
        }
    );
    // Complete the original response late. The client never resends the call or
    // attributes this late result to the next request on the shared connection.
    let end = Frame::message(Vec::new(), OpCode::Data(Data::Continue), true);
    peer.socket.send(Message::Frame(end)).await.unwrap();
    let next = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(1_000)).await })
    };
    let query = peer.read().await;
    assert_ne!(request["id"], query["id"]);
    peer.response(json!({"jsonrpc":"2.0","id":query["id"],"result":host.info(&peer.handle.caller()).unwrap()})).await;
    assert!(next.await.unwrap().is_ok());
    assert_eq!(
        host.info(&peer.handle.caller()).unwrap().control.revision,
        1
    );
}

#[tokio::test]
async fn slow_response_and_abandoned_waiter_do_not_block_other_calls_or_revoke_connection() {
    let (host, _) = fixture();
    let (client, mut peer) = delayed_peer(host.clone()).await;
    let slow = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(100)).await })
    };
    let slow_request = peer.read().await;
    let fast = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(1_000)).await })
    };
    let fast_request = peer.read().await;
    peer.response(json!({"jsonrpc":"2.0","id":fast_request["id"],"result":host.info(&peer.handle.caller()).unwrap()})).await;
    assert!(fast.await.unwrap().is_ok());
    assert_eq!(
        slow.await.unwrap().unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 100
        }
    );
    peer.response(json!({"jsonrpc":"2.0","id":slow_request["id"],"result":host.info(&peer.handle.caller()).unwrap()})).await;
    let abandoned = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(5_000)).await })
    };
    let abandoned_request = peer.read().await;
    abandoned.abort();
    let _ = abandoned.await;
    peer.response(json!({"jsonrpc":"2.0","id":abandoned_request["id"],"result":host.info(&peer.handle.caller()).unwrap()})).await;
    let final_call = {
        let client = client.clone();
        tokio::spawn(async move { client.info(options(1_000)).await })
    };
    let request = peer.read().await;
    peer.response(json!({"jsonrpc":"2.0","id":request["id"],"result":host.info(&peer.handle.caller()).unwrap()})).await;
    assert!(final_call.await.unwrap().is_ok());
    assert!(peer.handle.initial_info().can_operate);
}

#[tokio::test(start_paused = true)]
async fn initialization_budget_covers_tls_websocket_and_authenticated_response_once() {
    let (host, _) = fixture();
    host.configure_network_access(Some(FIRST.into())).unwrap();
    let (server_tls, client_tls) = wss_common::certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}/", listener.local_addr().unwrap().port());
    let started = tokio::time::Instant::now();
    let connecting = HostClient::wss(&url, client_tls, FIRST, options(200));
    tokio::pin!(connecting);
    assert!(futures_util::poll!(connecting.as_mut()).is_pending());
    let peer = async {
        let (tcp, _) = listener.accept().await.unwrap();
        tokio::time::advance(Duration::from_millis(60)).await;
        let tls = TlsAcceptor::from(server_tls).accept(tcp).await.unwrap();
        tokio::time::advance(Duration::from_millis(60)).await;
        let mut socket = accept_async_with_config(tls, None).await.unwrap();
        let Message::Text(auth) = socket.next().await.unwrap().unwrap() else {
            panic!("no initialization");
        };
        let auth: Value = serde_json::from_str(&auth).unwrap();
        assert_eq!(auth["method"], "initialize");
        let handle = host
            .connect_network(auth["params"]["access_key"].as_str().unwrap())
            .unwrap();
        (socket, handle)
    };
    tokio::pin!(peer);
    // Keep both real I/O futures progressing without Tokio auto-advancing the
    // paused clock while the OS is still delivering TLS/WebSocket bytes.
    let (_socket, _handle) = loop {
        tokio::select! {
            result = &mut connecting => panic!("initialization ended before authentication: {:?}", result.err()),
            authenticated = &mut peer => break authenticated,
            () = tokio::task::yield_now() => {}
        }
    };
    assert_eq!(started.elapsed(), Duration::from_millis(120));
    tokio::time::advance(Duration::from_millis(79)).await;
    assert!(futures_util::poll!(connecting.as_mut()).is_pending());
    tokio::time::advance(Duration::from_millis(1)).await;
    // Leave the authenticated socket open without a reply. The outer guard
    // fails a restarted budget instead of waiting for that later deadline.
    let result = tokio::time::timeout_at(started + Duration::from_millis(201), connecting)
        .await
        .expect("initialization must finish at the original deadline");
    assert_eq!(
        result.err().unwrap(),
        CallError::WaitTimeout {
            wait_timeout_ms: 200
        }
    );
    assert_eq!(started.elapsed(), Duration::from_millis(200));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_turn_wait_and_waiter_cancellation_leave_real_runtime_and_connection_live() {
    let (host, resources) = fixture();
    let server = Server::start(host).await;
    let client = server.client(FIRST).await.unwrap();
    let session = create(&client).await;
    session
        .send(input("real WSS Runtime execution"), options(500))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(&session).await;
    assert_eq!(
        session
            .wait_turn(turn.clone(), options(20))
            .await
            .unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 20
        }
    );
    let waiting = {
        let session = session.clone();
        let turn = turn.clone();
        tokio::spawn(async move { session.wait_turn(turn, options(10_000)).await })
    };
    assert!(client.info(options(500)).await.is_ok());
    assert_eq!(
        session
            .pause(control(), options(500))
            .await
            .unwrap()
            .result
            .scheduling,
        Scheduling::Paused
    );
    waiting.abort();
    let _ = waiting.await;
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
            .as_ref()
            .unwrap()
            .outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(client.info(options(500)).await.unwrap().control.revision, 1);
}

#[tokio::test]
async fn wss_preserves_unrecognized_faults_in_errors_and_accepted_results() {
    let (host, _) = fixture();
    let (client, mut peer) = delayed_peer(host.clone()).await;
    let unknown = json!({"type":"SESSION_FUTURE_WSS_FIXTURE","code":3599,"message":"retain actual receipt","details":{"future":true}});
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
    peer.response(json!({"jsonrpc":"2.0","id":failed_request["id"],"error":{"code":-32000,"message":"Business operation failed","data":unknown}})).await;
    peer.response(json!({"jsonrpc":"2.0","id":other_request["id"],"result":host.info(&peer.handle.caller()).unwrap()})).await;
    let CallError::Business(fault) = failing.await.unwrap().unwrap_err() else {
        panic!("wrong error provenance");
    };
    assert_eq!(value(fault), unknown);
    assert!(other.await.unwrap().is_ok());
    let created = host
        .create_agent_async(&peer.handle.caller(), create_request())
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    host.session_pause(&peer.handle.caller(), &target, control())
        .unwrap();
    let sending = {
        let session = client.session(target.clone());
        tokio::spawn(async move {
            session
                .send(input("accepted WSS input"), options(1_000))
                .await
        })
    };
    let request = peer.read().await;
    let accepted = host
        .send(
            &peer.handle.caller(),
            serde_json::from_value(request["params"].clone()).unwrap(),
        )
        .unwrap();
    let mut receipt = value(accepted.clone());
    receipt["result"]["start_error"] = unknown.clone();
    peer.response(json!({"jsonrpc":"2.0","id":request["id"],"result":receipt}))
        .await;
    let received = sending.await.unwrap().unwrap();
    assert_eq!(received.message_id, accepted.message_id);
    assert_eq!(received.result.delivery, Delivery::Accepted);
    assert_eq!(value(received.result.start_error.unwrap()), unknown);
    assert_eq!(
        host.session_state(&peer.handle.caller(), &target)
            .unwrap()
            .data
            .next_turn
            .len(),
        1
    );
}
