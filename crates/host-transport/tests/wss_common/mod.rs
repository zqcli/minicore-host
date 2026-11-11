#![allow(dead_code)]
//! Generated loopback-only TLS certificates. No persisted or real credentials.
use futures_util::{SinkExt, StreamExt};
use minicore_host_core::Host;
use minicore_host_protocol::*;
use minicore_host_transport::{HostClient, serve_wss, tls};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::{
    WebSocketStream, client_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

pub const FIRST: &str = "fixture-only-first-host-access-secret";
pub const SECOND: &str = "fixture-only-second-host-access-secret";

pub fn certificates() -> (Arc<tls::ServerConfig>, Arc<tls::ClientConfig>) {
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = generated.cert.der().clone();
    let key = tls::pki_types::PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der());
    let server = tls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key.into())
        .unwrap();
    let mut roots = tls::RootCertStore::empty();
    roots.add(cert).unwrap();
    let client = tls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (Arc::new(server), Arc::new(client))
}
pub struct Server {
    pub address: SocketAddr,
    pub client_tls: Arc<tls::ClientConfig>,
    pub task: JoinHandle<Result<(), CallError>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub async fn start(host: Arc<Host>) -> Self {
        host.configure_network_access(Some(FIRST.into())).unwrap();
        tls_server(host).await
    }
    pub fn url(&self) -> String {
        format!("wss://localhost:{}/", self.address.port())
    }
    pub async fn client(&self, key: &str) -> Result<HostClient, CallError> {
        HostClient::wss(
            &self.url(),
            self.client_tls.clone(),
            key,
            CallOptions {
                wait_timeout_ms: Some(5_000),
            },
        )
        .await
    }
    pub async fn raw(&self) -> RawPeer {
        let tcp = TcpStream::connect(self.address).await.unwrap();
        let tls = TlsConnector::from(self.client_tls.clone())
            .connect("localhost".try_into().unwrap(), tcp)
            .await
            .unwrap();
        let config = WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None)
            .max_write_buffer_size(usize::MAX);
        let (socket, _) = client_async_with_config(self.url(), tls, Some(config))
            .await
            .unwrap();
        RawPeer { socket }
    }
}
pub type TlsSocket = tokio_rustls::client::TlsStream<TcpStream>;
pub struct RawPeer {
    pub socket: WebSocketStream<TlsSocket>,
}
impl RawPeer {
    pub async fn write(&mut self, value: Value) {
        self.socket
            .send(Message::Text(serde_json::to_string(&value).unwrap().into()))
            .await
            .unwrap();
    }
    pub async fn read(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match self.socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => break serde_json::from_str(&text).unwrap(),
                    Message::Ping(_) | Message::Pong(_) => {}
                    other => panic!("unexpected frame: {other:?}"),
                }
            }
        })
        .await
        .unwrap()
    }
    pub async fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.write(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .await;
        self.read().await
    }
    pub async fn initialize(&mut self, key: &str) -> Value {
        self.request(0, "initialize", json!({"access_key":key}))
            .await
    }
    pub async fn assert_disconnected(&mut self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match self.socket.next().await {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
    }
}

/// Start the real TLS listener with the Host's already-published credentials.
pub async fn tls_server(host: Arc<Host>) -> Server {
    let (server_tls, client_tls) = certificates();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(serve_wss(host, listener, server_tls));
    Server {
        address,
        client_tls,
        task,
    }
}

/// Shared connection lifetimes for query parity across typed, OS pipes and WSS.
pub struct Clients {
    pub clients: [HostClient; 3],
    pub tls: Server,
    pub(super) pipe: JoinHandle<std::io::Result<()>>,
}
impl Clients {
    pub async fn new(host: Arc<Host>, key: &str, options: CallOptions) -> Self {
        let local = HostClient::in_process(host.clone()).unwrap();
        let (to_server, input) = crate::os_pipe::pipe().await.unwrap();
        let (output, from_server) = crate::os_pipe::pipe().await.unwrap();
        let pipe = tokio::spawn(minicore_host_transport::serve_stdio(
            host.clone(),
            input,
            output,
        ));
        let stdio = HostClient::stdio(from_server, to_server, options)
            .await
            .unwrap();
        let tls = tls_server(host).await;
        let wss = tls.client(key).await.unwrap();
        Self {
            clients: [local, stdio, wss],
            tls,
            pipe,
        }
    }
    pub async fn finish(self) {
        for client in &self.clients {
            client.close();
        }
        self.pipe.await.unwrap().unwrap();
    }
}
