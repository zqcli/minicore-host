#![cfg(unix)]
mod support;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_runtime::{LoopOptions, LoopOutcome};
use serde_json::{Value, json};
use std::{fs, sync::Arc};
use support::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn frame(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"explicit-http-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}
fn tool(id: &str, name: &str, args: Value) -> String {
    frame(
        json!({"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]}),
        None,
    ) + &frame(json!({}), Some("tool_calls"))
        + "data: [DONE]\n\n"
}
async fn request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let boundary = loop {
        if let Some(index) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break index + 4;
        }
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        bytes.extend(&buffer[..count]);
    };
    let header = std::str::from_utf8(&bytes[..boundary]).unwrap();
    let length: usize = header
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap();
    while bytes.len() < boundary + length {
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        bytes.extend(&buffer[..count]);
    }
    serde_json::from_slice(&bytes[boundary..boundary + length]).unwrap()
}

/// Scripted loopback HTTP/SSE replies exercise the production provider and
/// actual Runtime plus local tools. This is not a live model-provider claim.
#[tokio::test]
async fn production_http_provider_round_trip_executes_real_files_and_shell() {
    let (root, tools, records) = fixture().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        let replies = [
            tool(
                "http-write",
                "write",
                json!({"path":"from-http.txt","content":"actual filesystem text"}),
            ),
            tool(
                "http-bash",
                "bash",
                json!({"command":"cat from-http.txt; printf observed-stderr >&2; exit 7"}),
            ),
            frame(json!({"content":"Verified the actual tool results"}), None)
                + &frame(json!({}), Some("stop"))
                + "data: [DONE]\n\n",
        ];
        let mut requests = Vec::new();
        for body in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(request(&mut socket).await);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        requests
    });
    let model = Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(endpoint, "scripted-http-model", descriptor()),
            SecretApiKey::new("fixture-only-key").unwrap(),
        )
        .unwrap(),
    );
    let report = finish(
        start(
            model,
            &tools,
            &["write", "bash"],
            LoopOptions::default_checked().unwrap(),
        ),
        &tools,
    )
    .await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    assert_eq!(
        fs::read_to_string(root.path().join("from-http.txt")).unwrap(),
        "actual filesystem text"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 2);
    let messages = requests[2]["messages"].as_array().unwrap();
    let bash = messages
        .iter()
        .find(|message| message["tool_call_id"] == "http-bash")
        .unwrap();
    let result: Value = serde_json::from_str(bash["content"].as_str().unwrap()).unwrap();
    assert_eq!(result["exit_code"], 7);
    assert_eq!(result["stdout"]["output"]["data"], "actual filesystem text");
    assert_eq!(result["stderr"]["output"]["data"], "observed-stderr");
    assert_eq!(records.edits.lock().unwrap().len(), 1);
    assert_eq!(records.commands.lock().unwrap().len(), 1);
}
