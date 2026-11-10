//! Loopback-only HTTP provider fixture. This is a scripted test server, not a
//! model or Runtime substitute. Every request traverses reqwest and TCP.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[path = "../../../../tests/support/http.rs"]
mod http;
pub use http::Captured;
use http::read_request;

pub enum Reply {
    Full {
        status: u16,
        content_type: &'static str,
        headers: String,
        body: String,
        fragment: usize,
    },
    Hold {
        prefix: String,
        written: Arc<Notify>,
        disconnected: Arc<Notify>,
    },
    HeadersHold {
        received: Arc<Notify>,
    },
}

impl Reply {
    pub fn sse(body: impl Into<String>) -> Self {
        Self::Full {
            status: 200,
            content_type: "text/event-stream; charset=utf-8",
            headers: String::new(),
            body: body.into(),
            fragment: 7,
        }
    }
    pub fn error(status: u16, body: impl Into<String>) -> Self {
        Self::Full {
            status,
            content_type: "application/json",
            headers: String::new(),
            body: body.into(),
            fragment: 0,
        }
    }
}

pub struct Fixture {
    pub endpoint: String,
    requests: Arc<Mutex<Vec<Captured>>>,
    shutdown: CancellationToken,
    join: JoinHandle<()>,
}

impl Fixture {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let shutdown = CancellationToken::new();
        let cancellation = shutdown.clone();
        let mut replies: VecDeque<_> = replies.into();
        let join = tokio::spawn(async move {
            loop {
                let (mut socket, _) = tokio::select! {
                    _ = cancellation.cancelled() => break,
                    connection = listener.accept() => connection.unwrap(),
                };
                let response = async {
                    let request = read_request(&mut socket).await;
                    captured.lock().unwrap().push(request);
                    let reply = replies
                        .pop_front()
                        .unwrap_or_else(|| Reply::error(500, "{}"));
                    send_reply(&mut socket, reply).await;
                };
                tokio::select! { _ = cancellation.cancelled() => break, _ = response => {} }
            }
        });
        Self {
            endpoint,
            requests,
            shutdown,
            join,
        }
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub async fn finish(self) -> Vec<Captured> {
        self.shutdown.cancel();
        self.join.await.unwrap();
        self.requests.lock().unwrap().clone()
    }
}

async fn send_reply(socket: &mut TcpStream, reply: Reply) {
    match reply {
        Reply::Full {
            status,
            content_type,
            headers,
            body,
            fragment,
        } => {
            let head = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
                body.len()
            );
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            for part in body.as_bytes().chunks(if fragment == 0 {
                body.len().max(1)
            } else {
                fragment
            }) {
                if socket.write_all(part).await.is_err() {
                    return;
                }
                tokio::task::yield_now().await;
            }
            let _ = socket.shutdown().await;
        }
        Reply::Hold {
            prefix,
            written,
            disconnected,
        } => {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
            socket.write_all(prefix.as_bytes()).await.unwrap();
            written.notify_one();
            let mut byte = [0u8];
            let result = socket.read(&mut byte).await;
            assert!(
                matches!(result, Ok(0) | Err(_)),
                "unexpected request bytes during stream"
            );
            disconnected.notify_one();
        }
        Reply::HeadersHold { received } => {
            received.notify_one();
            let mut byte = [0u8];
            let _ = socket.read(&mut byte).await;
        }
    }
}
