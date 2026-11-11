//! Real OS pipes with independently controlled request and response lifetimes.
use minicore_host_core::Host;
use minicore_host_protocol::CallOptions;
use minicore_host_transport::{HostClient, serve_stdio};
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, DuplexStream, ReadBuf},
    sync::oneshot,
    task::JoinHandle,
};

pub struct PipeServer {
    pub requests: JoinHandle<io::Result<u64>>,
    pub server: JoinHandle<io::Result<()>>,
}
impl PipeServer {
    pub async fn close(self, client: &HostClient, wait: Option<Duration>) {
        // Close only the request endpoint first. The actual response reader
        // stays alive until EOF has retired the server and its writer.
        self.requests.abort();
        assert!(self.requests.await.unwrap_err().is_cancelled());
        let joined = match wait {
            Some(wait) => tokio::time::timeout(wait, self.server).await.unwrap(),
            None => self.server.await,
        };
        joined.unwrap().unwrap();
        client.close();
    }
}
/// Copy request bytes unchanged; aborting the relay closes the real write end.
pub fn request_relay(
    mut to_server: crate::os_pipe::Sender,
) -> (DuplexStream, JoinHandle<io::Result<u64>>) {
    let (writer, mut requests) = tokio::io::duplex(4096);
    let requests =
        tokio::spawn(async move { tokio::io::copy(&mut requests, &mut to_server).await });
    (writer, requests)
}
pub async fn stdio(host: Arc<Host>, options: CallOptions) -> (HostClient, PipeServer) {
    let (to_server, input) = crate::os_pipe::pipe().await.unwrap();
    let (output, from_server) = crate::os_pipe::pipe().await.unwrap();
    let (writer, requests) = request_relay(to_server);
    let server = tokio::spawn(serve_stdio(host, input, output));
    let client = HostClient::stdio(from_server, writer, options)
        .await
        .unwrap();
    (client, PipeServer { requests, server })
}

pub struct ReaderLifetime<R> {
    pub reader: Option<R>,
    pub dropped: Option<oneshot::Sender<()>>,
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
        // The signal follows the actual OS endpoint drop, not just task abort.
        self.reader.take();
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}
