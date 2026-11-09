//! TLS/WebSocket framing only; RPC semantics and ownership stay shared.
use crate::rpc::{FrameReader, FrameWriter, Initialization, RpcClient, serve_frames};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use minicore_host_core::Host;
use minicore_host_protocol::CallError;
use std::{io, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};
use tokio_rustls::{
    TlsAcceptor, TlsConnector,
    rustls::{ClientConfig, ServerConfig, pki_types::ServerName},
};
use tokio_tungstenite::{
    WebSocketStream, accept_async_with_config, client_async_with_config,
    tungstenite::{Error, Message, client::IntoClientRequest, protocol::WebSocketConfig},
};

/// RFC/TLS validation remains enabled. The application deliberately imposes no
/// frame, message, or write-buffer quota, including the library's defaults.
fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .write_buffer_size(0)
        .max_write_buffer_size(usize::MAX)
        .max_message_size(None)
        .max_frame_size(None)
}

struct WebSocketReader<S>(SplitStream<WebSocketStream<S>>);
struct WebSocketWriter<S>(SplitSink<WebSocketStream<S>, Message>);
fn split<S: AsyncRead + AsyncWrite + Unpin>(
    socket: WebSocketStream<S>,
) -> (WebSocketReader<S>, WebSocketWriter<S>) {
    let (writer, reader) = socket.split();
    (WebSocketReader(reader), WebSocketWriter(writer))
}
impl<S: AsyncRead + AsyncWrite + Unpin + Send> FrameReader for WebSocketReader<S> {
    async fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        while let Some(message) = self.0.next().await {
            match message.map_err(websocket_error)? {
                Message::Text(text) => return Ok(Some(text.as_bytes().to_vec())),
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Binary(_) | Message::Frame(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "JSON-RPC requires a WebSocket text message",
                    ));
                }
            }
        }
        Ok(None)
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin + Send> FrameWriter for WebSocketWriter<S> {
    async fn write(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let text = String::from_utf8(bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "JSON-RPC output is not UTF-8")
        })?;
        self.0
            .send(Message::Text(text.into()))
            .await
            .map_err(websocket_error)
    }
    async fn close(&mut self) -> io::Result<()> {
        self.0.close().await.map_err(websocket_error)
    }
}
fn websocket_error(error: Error) -> io::Error {
    // Do not expose protocol frame contents, headers or authentication input.
    match error {
        Error::Io(error) => error,
        Error::ConnectionClosed | Error::AlreadyClosed => io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "WebSocket connection closed",
        ),
        _ => io::Error::new(io::ErrorKind::InvalidData, "WebSocket protocol failure"),
    }
}
fn connection_error(error: impl std::fmt::Display) -> CallError {
    CallError::Connection {
        message: error.to_string(),
    }
}
fn protocol(message: &str) -> CallError {
    CallError::Protocol {
        message: message.into(),
    }
}

/// Serve authenticated WSS over a listener and TLS configuration supplied by a
/// trusted embedding process. Host network access must be explicitly configured
/// before this call. This does not assemble a process, load credentials, expose
/// HTTP-RPC, or invent default network settings.
///
/// Every connection, including loopback, must send the setup request
/// `initialize` with exactly `{ "access_key": "..." }`. Authentication and
/// registration are one Host boundary; no business reads or subscriptions are
/// dispatched beforehand. Dropping this server closes its connections, while
/// work already accepted by business owners continues.
pub async fn serve_wss(
    host: Arc<Host>,
    listener: TcpListener,
    tls: Arc<ServerConfig>,
) -> Result<(), CallError> {
    host.require_network_access()?;
    let acceptor = TlsAcceptor::from(tls);
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (socket, _) = accepted.map_err(connection_error)?;
                let host = Arc::clone(&host);
                let acceptor = acceptor.clone();
                connections.spawn(async move {
                    let tls = acceptor.accept(socket).await?;
                    let socket = accept_async_with_config(tls, Some(websocket_config())).await.map_err(websocket_error)?;
                    let (reader, writer) = split(socket);
                    serve_frames(host, reader, writer, Initialization::Network).await
                });
            }
            Some(joined) = connections.join_next(), if !connections.is_empty() => {
                // A peer's malformed handshake or disconnect belongs to that
                // connection. It never disables access for other peers.
                let _connection_result = joined.map_err(|_| connection_error("WSS connection task terminated unexpectedly"))?;
            }
        }
    }
}

pub(crate) async fn connect(
    url: &str,
    tls: Arc<ClientConfig>,
    access_key: &str,
) -> Result<RpcClient, CallError> {
    let request = url
        .into_client_request()
        .map_err(|_| protocol("invalid WSS address"))?;
    if request.uri().scheme_str() != Some("wss")
        || request
            .uri()
            .authority()
            .is_some_and(|authority| authority.as_str().contains('@'))
    {
        return Err(protocol(
            "a WSS address without embedded credentials is required",
        ));
    }
    let hostname = request
        .uri()
        .host()
        .ok_or_else(|| protocol("WSS address requires a host"))?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = request.uri().port_u16().unwrap_or(443);
    let server_name =
        ServerName::try_from(hostname.clone()).map_err(|_| protocol("invalid TLS server name"))?;
    let socket = TcpStream::connect((hostname.as_str(), port))
        .await
        .map_err(connection_error)?;
    let tls = TlsConnector::from(tls)
        .connect(server_name, socket)
        .await
        .map_err(connection_error)?;
    let (socket, _) = client_async_with_config(request, tls, Some(websocket_config()))
        .await
        .map_err(|error| connection_error(websocket_error(error)))?;
    let (reader, writer) = split(socket);
    let client = RpcClient::start_frames(reader, writer);
    let initial = client.authenticate(access_key).await?;
    Ok(client.initialized(initial))
}

#[cfg(test)]
mod tests {
    #[test]
    fn websocket_quotas_are_disabled_without_weakening_protocol_checks() {
        let config = super::websocket_config();
        assert_eq!(config.max_frame_size, None);
        assert_eq!(config.max_message_size, None);
        assert_eq!(config.max_write_buffer_size, usize::MAX);
        assert!(!config.accept_unmasked_frames);
    }
}
