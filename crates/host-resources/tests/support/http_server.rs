//! Sequential loopback responses used by resource and local-tool scenarios.
#[path = "../../../../tests/support/http.rs"]
pub(crate) mod http;

use serde_json::Value;
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc},
    task::JoinHandle,
};

pub async fn start(
    replies: Vec<(String, Option<Arc<Semaphore>>)>,
) -> (String, mpsc::UnboundedReceiver<Value>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let (send, requests) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        for (body, gate) in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            send.send(http::read_request(&mut socket).await.body)
                .unwrap();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            http::write_sse(&mut socket, &body).await;
        }
    });
    (endpoint, requests, task)
}
