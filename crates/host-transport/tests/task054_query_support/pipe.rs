//! Test-only pipe lifetimes and a gated proof of the SDK drop-cleanup race.
//! Request bytes are copied unchanged; Host, Runtime, SDK and writes are real.
use crate::common::*;
use minicore_host_core::Host;
use minicore_host_protocol::*;
use minicore_host_transport::{HostClient, serve_stdio};
use serde_json::Value;
use std::{
    future::Future,
    io,
    os::fd::AsFd,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, ready},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::oneshot,
    task::JoinHandle,
};

pub struct PipeServer {
    requests: JoinHandle<io::Result<u64>>,
    server: JoinHandle<io::Result<()>>,
}
impl PipeServer {
    pub async fn close(self, client: &HostClient) {
        // Close the real request write end first. The response reader stays
        // alive until the actual server observes EOF and its writer is gone.
        self.requests.abort();
        assert!(self.requests.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(3), self.server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        client.close();
    }
}
pub async fn stdio(host: Arc<Host>) -> (HostClient, PipeServer) {
    let (mut to_server, input) = tokio::net::unix::pipe::pipe().unwrap();
    let (output, from_server) = tokio::net::unix::pipe::pipe().unwrap();
    let (writer, mut requests) = tokio::io::duplex(4096);
    let requests =
        tokio::spawn(async move { tokio::io::copy(&mut requests, &mut to_server).await });
    let server = tokio::spawn(serve_stdio(host, input, output));
    let client = HostClient::stdio(from_server, writer, options(2000))
        .await
        .unwrap();
    (client, PipeServer { requests, server })
}

struct GatedWriter<W> {
    output: W,
    armed: Arc<AtomicBool>,
    entered: Option<oneshot::Sender<Vec<u8>>>,
    release: Option<oneshot::Receiver<()>>,
}
impl<W: AsyncWrite + Unpin> AsyncWrite for GatedWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.armed.load(Ordering::SeqCst) {
            if let Some(entered) = self.entered.take() {
                entered.send(bytes.to_vec()).unwrap();
            }
            if let Some(release) = &mut self.release {
                ready!(Pin::new(release).poll(cx)).expect("release the actual write");
                self.release = None;
            }
        }
        Pin::new(&mut self.output).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.output).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.output).poll_shutdown(cx)
    }
}
struct ReaderLifetime<R> {
    reader: Option<R>,
    dropped: Option<oneshot::Sender<()>>,
}
impl<R: AsyncRead + Unpin> AsyncRead for ReaderLifetime<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.reader.as_mut().unwrap()).poll_read(cx, bytes)
    }
}
impl<R> Drop for ReaderLifetime<R> {
    fn drop(&mut self) {
        // Signal only after the actual OS read endpoint has been dropped.
        self.reader.take();
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sdk_drop_cleanup_reply_proves_both_pipe_close_orders() {
    for reader_first in [true, false] {
        let (host, resources) = fixture();
        let owner = HostClient::in_process(host.clone()).unwrap();
        let session = create(&owner).await;
        session
            .send(input("real work survives observer cleanup"), options(2000))
            .await
            .unwrap();
        resources.model.started.acquire().await.unwrap().forget();
        let turn = running(&session).await;
        let (mut to_server, input) = tokio::net::unix::pipe::pipe().unwrap();
        // This duplicate keeps request EOF under test control even when the
        // SDK aborts its writer. No bytes are written through the duplicate.
        let request_lifetime = to_server.as_fd().try_clone_to_owned().unwrap();
        let (output, from_server) = tokio::net::unix::pipe::pipe().unwrap();
        let (writer, mut requests) = tokio::io::duplex(4096);
        let requests =
            tokio::spawn(async move { tokio::io::copy(&mut requests, &mut to_server).await });
        let armed = Arc::new(AtomicBool::new(false));
        let (entered, write_started) = oneshot::channel();
        let (release, gate) = oneshot::channel();
        let output = GatedWriter {
            output,
            armed: armed.clone(),
            entered: Some(entered),
            release: Some(gate),
        };
        let server = tokio::spawn(serve_stdio(host, input, output));
        let (dropped, reader_closed) = oneshot::channel();
        let client = HostClient::stdio(
            ReaderLifetime {
                reader: Some(from_server),
                dropped: Some(dropped),
            },
            writer,
            options(2000),
        )
        .await
        .unwrap();
        let subscription = client
            .subscribe(
                SubscribeRequest {
                    scope: SubscriptionScope::Turn {
                        session_id: session.target().session_id.clone(),
                        turn_id: turn.clone(),
                    },
                    // The model remains gated, so no completion can compete
                    // with the cleanup reply whose real bytes we intercept.
                    categories: [EventCategory::Completion].into_iter().collect(),
                },
                options(2000),
            )
            .await
            .unwrap();
        assert!(
            client
                .unsubscribe(
                    UnsubscribeRequest {
                        subscription_id: subscription.result().subscription_id.clone(),
                    },
                    options(2000),
                )
                .await
                .unwrap()
                .ended
        );
        armed.store(true, Ordering::SeqCst);
        drop(subscription);
        // No request was made by this test after arming. These are the real
        // second unsubscribe's reply bytes caused solely by SDK Drop cleanup.
        let bytes = tokio::time::timeout(Duration::from_secs(3), write_started)
            .await
            .unwrap()
            .unwrap();
        let reply: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(reply["id"].is_number());
        assert_eq!(reply["result"]["ended"], false);
        if reader_first {
            client.close();
            tokio::time::timeout(Duration::from_secs(3), reader_closed)
                .await
                .unwrap()
                .unwrap();
            release.send(()).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), server)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::BrokenPipe
            );
            drop(request_lifetime);
            requests.await.unwrap().unwrap();
            eprintln!("SDK drop cleanup: ended=false, response reader first => BrokenPipe");
        } else {
            drop(request_lifetime);
            PipeServer { requests, server }.close(&client).await;
            assert!(release.send(()).is_err(), "EOF must retire the held writer");
            eprintln!("SDK drop cleanup: ended=false, request EOF first => clean server join");
        }
        resources.model.finish.add_permits(1);
        let result = session.wait_turn(turn, options(2000)).await.unwrap();
        assert_eq!(result.data.info.outcome, Some(TurnOutcome::Completed));
        assert!(
            value(&result.data.history)
                .to_string()
                .contains("fixture output")
        );
        assert!(owner.info(options(2000)).await.is_ok());
    }
}
