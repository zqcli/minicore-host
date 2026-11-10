//! Test-only pipe lifetimes for the retained-query fixture.
//! The relay copies requests unchanged; Host, Runtime, SDK and pipes are real.
use crate::{assert_retained, common::*, original_terminal};
use minicore_host_core::Host;
use minicore_host_protocol::TurnOutcome;
use minicore_host_transport::{HostClient, serve_stdio};
use serde_json::Value;
use std::{
    future::Future,
    io,
    os::fd::AsFd,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
    time::Duration,
};
use tokio::{io::AsyncWrite, sync::oneshot};

use crate::common::pipes::{PipeServer, ReaderLifetime, request_relay};

pub async fn stdio_pipes(host: Arc<Host>) -> (HostClient, PipeServer) {
    crate::common::pipes::stdio(host, options(5000)).await
}

struct CleanupWriter<W> {
    output: W,
    entered: Option<oneshot::Sender<Vec<u8>>>,
    release: Option<oneshot::Receiver<()>>,
    holding: bool,
}
impl<W: AsyncWrite + Unpin> AsyncWrite for CleanupWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.entered.is_some()
            && serde_json::from_slice::<Value>(bytes)
                .is_ok_and(|reply| reply["result"]["ended"] == false)
        {
            self.holding = true;
            self.entered.take().unwrap().send(bytes.to_vec()).unwrap();
        }
        if self.holding {
            ready!(Pin::new(self.release.as_mut().unwrap()).poll(cx))
                .expect("release the actual cleanup write");
            self.release = None;
            self.holding = false;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retained_query_cleanup_proves_both_pipe_close_orders() {
    for reader_first in [true, false] {
        let (host, resources) = fixture();
        let owner = HostClient::in_process(host.clone()).unwrap();
        let session = create(&owner).await;
        let original = session.info(options(5000)).await.unwrap().config;
        // Settle the original Turn through its real owner. Only assert_retained
        // below creates a subscription on this new pipe connection.
        let (finished, _) = original_terminal(&session, &owner, &resources).await;
        let (to_server, server_input) = tokio::net::unix::pipe::pipe().unwrap();
        // Keep request EOF under test control even if SDK close aborts its
        // writer. This duplicate never writes bytes or generates requests.
        let request_lifetime = to_server.as_fd().try_clone_to_owned().unwrap();
        let (output, from_server) = tokio::net::unix::pipe::pipe().unwrap();
        let (writer, requests) = request_relay(to_server);
        let (entered, write_started) = oneshot::channel();
        let (release, gate) = oneshot::channel();
        let output = CleanupWriter {
            output,
            entered: Some(entered),
            release: Some(gate),
            holding: false,
        };
        let server = tokio::spawn(serve_stdio(host, server_input, output));
        let (dropped, reader_closed) = oneshot::channel();
        let client = HostClient::stdio(
            ReaderLifetime {
                reader: Some(from_server),
                dropped: Some(dropped),
            },
            writer,
            options(5000),
        )
        .await
        .unwrap();
        let observed = client.session(session.target().clone());
        // Exercise the unmodified canonical query assertions and their exact
        // explicit-unsubscribe/Drop sequence, not a handwritten substitute.
        assert_retained(&client, &observed, &finished, &original).await;
        let bytes = tokio::time::timeout(Duration::from_secs(5), write_started)
            .await
            .unwrap()
            .unwrap();
        let reply: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(reply["id"].is_number());
        assert_eq!(reply["result"]["ended"], false);
        if reader_first {
            client.close();
            tokio::time::timeout(Duration::from_secs(5), reader_closed)
                .await
                .unwrap()
                .unwrap();
            release.send(()).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(5), server)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::BrokenPipe
            );
            drop(request_lifetime);
            requests.await.unwrap().unwrap();
            eprintln!("assert_retained cleanup: ended=false, response reader first => BrokenPipe");
        } else {
            drop(request_lifetime);
            PipeServer { requests, server }
                .close(&client, Some(Duration::from_secs(5)))
                .await;
            assert!(
                release.send(()).is_err(),
                "request EOF must retire the held writer"
            );
            eprintln!(
                "assert_retained cleanup: ended=false, request EOF first => clean server join"
            );
        }
        // Observer teardown neither mutates the original terminal snapshot nor
        // prevents the existing owner from accepting and completing real work.
        assert_retained(&owner, &session, &finished, &original).await;
        session
            .send(input("work after observer teardown"), options(5000))
            .await
            .unwrap();
        resources.model.started.acquire().await.unwrap().forget();
        let next = running(&session).await;
        resources.model.finish.add_permits(1);
        let result = session.wait_turn(next, options(5000)).await.unwrap();
        assert_eq!(
            result.data.info.as_ref().unwrap().outcome,
            Some(TurnOutcome::Completed)
        );
        assert_retained(&owner, &session, &finished, &original).await;
    }
}
