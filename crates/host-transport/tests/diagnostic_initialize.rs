//! Real Host setup over OS pipes and authenticated loopback TLS/WebSocket.
//! A test wire relay holds the actual response, not a fabricated Host event.
mod diagnostic_support;
mod os_pipe;
mod wss_common;
use diagnostic_support::{WAIT, host, stop};
use futures_util::{SinkExt, StreamExt};
use minicore_host_core::Host;
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::{CallError, CallOptions};
use minicore_host_transport::{HostClient, serve_stdio};
use serde_json::Value;
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
    time::Duration,
};
use tokio::{io::AsyncWrite, net::TcpListener, sync::oneshot, task::JoinHandle};

struct GateWriter<W> {
    inner: W,
    entered: Option<oneshot::Sender<Vec<u8>>>,
    release: oneshot::Receiver<()>,
}
impl<W: AsyncWrite + Unpin> AsyncWrite for GateWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(entered) = self.entered.take() {
            entered.send(bytes.to_vec()).unwrap();
        }
        ready!(Pin::new(&mut self.release).poll(cx)).expect("release only for a completed write");
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
fn options(abandoned: bool) -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(if abandoned { 5_000 } else { 100 }),
    }
}
async fn finish(client: JoinHandle<Result<HostClient, CallError>>, abandoned: bool) {
    if abandoned {
        client.abort();
        assert!(client.await.is_err_and(|error| error.is_cancelled()));
    } else {
        let result = tokio::time::timeout(WAIT, client).await.unwrap().unwrap();
        assert!(matches!(
            result,
            Err(CallError::WaitTimeout {
                wait_timeout_ms: 100
            })
        ));
    }
}
async fn stdio(host: Arc<Host>, abandoned: bool) {
    let (send, input) = crate::os_pipe::pipe().await.unwrap();
    let (output, receive) = crate::os_pipe::pipe().await.unwrap();
    let (entered, response) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let server = tokio::spawn(serve_stdio(
        host,
        input,
        GateWriter {
            inner: output,
            entered: Some(entered),
            release: released,
        },
    ));
    let client = tokio::spawn(HostClient::stdio(receive, send, options(abandoned)));
    let response = tokio::time::timeout(WAIT, response).await.unwrap().unwrap();
    let response: Value = serde_json::from_slice(&response).unwrap();
    assert!(response["result"]["host_id"].is_string());
    finish(client, abandoned).await;
    // Request EOF ends the actual server without pretending the response sent.
    tokio::time::timeout(WAIT, server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(release);
}
async fn wss(host: Arc<Host>, abandoned: bool) {
    let server = wss_common::Server::start(host).await;
    let (server_tls, client_tls) = wss_common::certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}/", listener.local_addr().unwrap().port());
    let (entered, response) = oneshot::channel();
    // This wire relay connects both real TLS/WebSocket ends. Only its final
    // write is gated; the production Host authenticates the unchanged request.
    let mut upstream = server.raw().await;
    let relay = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(server_tls)
            .accept(socket)
            .await
            .unwrap();
        let mut socket = tokio_tungstenite::accept_async(tls).await.unwrap();
        let request = socket.next().await.unwrap().unwrap();
        upstream.socket.send(request).await.unwrap();
        let response = upstream.read().await;
        assert!(response["result"]["host_id"].is_string());
        entered.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let client = tokio::spawn(async move {
        HostClient::wss(&url, client_tls, wss_common::FIRST, options(abandoned)).await
    });
    tokio::time::timeout(WAIT, response).await.unwrap().unwrap();
    finish(client, abandoned).await;
    relay.abort();
    assert!(relay.await.is_err_and(|error| error.is_cancelled()));
    drop(server);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialization_deadline_reports_timeout_and_real_drop_separately_on_stdio_and_wss() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("initialize.jsonl");
    let mut logging = Options::new("diagnostic-initialize");
    logging.level = tracing::Level::DEBUG;
    logging.output = Output::File(path.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let host = host("http://127.0.0.1:1/v1/chat/completions").await;
    for abandoned in [false, true] {
        stdio(host.clone(), abandoned).await;
        wss(host.clone(), abandoned).await;
    }
    stop(&host).await;
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains(wss_common::FIRST));
    let records: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let client: Vec<_> = records
        .iter()
        .filter(|row| {
            row["component"] == "client.rpc" && row["attributes"]["method"] == "initialize"
        })
        .collect();
    assert_eq!(client.len(), 4);
    for (row, outcome) in client.iter().zip([
        "wait_timeout",
        "wait_timeout",
        "wait_abandoned",
        "wait_abandoned",
    ]) {
        assert_eq!(row["attributes"]["outcome"], outcome);
        assert_eq!(row["attributes"]["delivery"], "unconfirmed");
        assert!(row.get("host_id").is_none());
        assert!(row["attributes"].get("target_host_id").is_none());
        if outcome == "wait_timeout" {
            assert_eq!(row["attributes"]["wait_timeout_ms"], 100);
        } else {
            assert!(row["attributes"].get("wait_timeout_ms").is_none());
        }
    }
}
