//! Deterministic OS-pipe teardown orders against a real Host and Runtime.
mod common;
mod observation_common;
mod os_pipe;
use common::*;
use minicore_host_protocol::*;
use minicore_host_runtime::model::ModelEvent;
use minicore_host_transport::*;
use observation_common::*;
use serde_json::{Value, json};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, ready},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    sync::oneshot,
};

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_eof_before_reader_close_is_clean_and_broken_reader_does_not_cancel_owner() {
    for reader_first in [true, false] {
        let (host, events, mut started) = setup(2, None);
        let owner = HostClient::in_process(host.clone()).unwrap();
        let session = create(&owner).await;
        let (mut to_server, server_input) = crate::os_pipe::pipe().await.unwrap();
        let (output, from_server) = crate::os_pipe::pipe().await.unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let (entered, write_started) = oneshot::channel();
        let (release, gate) = oneshot::channel();
        let output = GatedWriter {
            output,
            armed: armed.clone(),
            entered: Some(entered),
            release: Some(gate),
        };
        let server = tokio::spawn(serve_stdio(host, server_input, output));
        let mut reader = BufReader::new(from_server);
        let mut line = String::new();
        to_server
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
            .await
            .unwrap();
        reader.read_line(&mut line).await.unwrap();
        assert!(serde_json::from_str::<Value>(&line).unwrap()["result"].is_object());
        let subscribe = json!({
            "jsonrpc":"2.0", "id":2, "method":"events.subscribe",
            "params":request(scope(session.target()), &[EventCategory::Completion]),
        });
        to_server
            .write_all(format!("{subscribe}\n").as_bytes())
            .await
            .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let subscription_id =
            serde_json::from_str::<Value>(&line).unwrap()["result"]["subscription_id"].clone();
        assert!(subscription_id.is_string());

        session
            .send(input("before disconnect"), options(2000))
            .await
            .unwrap();
        started.recv().await.unwrap();
        events[0]
            .send(ModelEvent::text_delta("saved before close"))
            .unwrap();
        finish(&events[0]);
        // Drain the actual first Turn completion. No completion frame remains
        // to confuse the following gate with the unsubscribe response.
        loop {
            line.clear();
            assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["params"]["event"]["kind"] == "turn.finished" {
                break;
            }
        }
        session
            .send(input("accepted before disconnect"), options(2000))
            .await
            .unwrap();
        started.recv().await.unwrap();
        let turn = running(&session).await;
        armed.store(true, Ordering::SeqCst);
        let unsubscribe = json!({
            "jsonrpc":"2.0", "id":3, "method":"events.unsubscribe",
            "params":{"subscription_id":subscription_id},
        });
        to_server
            .write_all(format!("{unsubscribe}\n").as_bytes())
            .await
            .unwrap();
        let reply: Value = serde_json::from_slice(&write_started.await.unwrap()).unwrap();
        assert_eq!(reply["id"], 3);
        assert_eq!(reply["result"]["ended"], true);
        if reader_first {
            // The request pipe remains open, so EOF cannot race the known
            // response write. Closing its reader yields the exact OS error.
            drop(reader);
            release.send(()).unwrap();
            assert_eq!(
                server.await.unwrap().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            drop(to_server);
        } else {
            // Normal teardown keeps the response reader alive until request
            // EOF has ended the server. No I/O error is ignored or retried.
            drop(to_server);
            server.await.unwrap().unwrap();
            drop(reader);
            assert!(release.send(()).is_err());
        }
        events[1]
            .send(ModelEvent::text_delta("completed after disconnect"))
            .unwrap();
        finish(&events[1]);
        assert_eq!(
            session
                .wait_turn(turn, options(2000))
                .await
                .unwrap()
                .data
                .info
                .as_ref()
                .unwrap()
                .outcome,
            Some(TurnOutcome::Completed)
        );
        let history = session
            .history(
                HistoryReadRequest {
                    direction: minicore_host_protocol::HistoryDirection::Forward,
                    cursor: None,
                    limit: 100,
                },
                options(2000),
            )
            .await
            .unwrap();
        let history = serde_json::to_string(&history.data.items).unwrap();
        assert!(history.contains("saved before close"));
        assert!(history.contains("completed after disconnect"));
        assert!(owner.info(options(2000)).await.is_ok());
    }
}
