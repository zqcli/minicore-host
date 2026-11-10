//! Loopback HTTP primitives shared by scripted provider fixtures. Scenarios
//! retain control of response fragmentation, gates, cancellation and shutdown.
#![allow(dead_code)] // Each integration target uses only its relevant helpers.

use serde_json::Value;
use std::time::Instant;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[derive(Clone, Debug)]
pub struct Captured {
    pub received_at: Instant,
    pub head: String,
    pub body: Value,
}

/// Read one complete Content-Length JSON request from the actual TCP stream.
pub async fn read_request(socket: &mut TcpStream) -> Captured {
    let mut bytes = Vec::new();
    let boundary = loop {
        if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break at + 4;
        }
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0, "HTTP fixture request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
    };
    let head = String::from_utf8(bytes[..boundary].to_vec()).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .expect("HTTP fixture request has Content-Length");
    while bytes.len() < boundary + length {
        let mut chunk = [0; 4096];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0, "HTTP fixture request ended before its body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    Captured {
        received_at: Instant::now(),
        head,
        body: serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap(),
    }
}

pub fn sse_headers(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
}

/// Send a complete ordinary SSE response. Fragmented/chunked scenarios write
/// their own frames and keep their original error and cancellation assertions.
pub async fn write_sse(socket: &mut TcpStream, body: &str) {
    socket
        .write_all(sse_headers(body).as_bytes())
        .await
        .unwrap();
    socket.write_all(body.as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
}
