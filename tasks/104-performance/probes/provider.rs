//! Copy unchanged to host-provider/examples/task104_provider.rs for a manual
//! release probe. These path modules compile the actual source encoder; no
//! alternate encoder is embedded here. This measures local request preparation,
//! not HTTP transport, model latency or a whole Host.
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;
#[path = "../src/request.rs"]
mod request;
#[path = "../src/sse.rs"]
mod sse;
#[allow(dead_code)]
#[path = "../src/stream.rs"]
mod stream;

use minicore_runtime::model::{
    AssistantPart, ModelDescriptor, ModelLimits, ModelMessage, ModelRequest, ReasoningPreference,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, hint::black_box, time::Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let bytes: usize = args[1].parse().unwrap();
    let iterations: usize = args[2].parse().unwrap();
    assert!(bytes > 0 && iterations > 0);
    let text = "a\"\\\n".repeat(bytes.div_ceil(4));
    let mut messages = vec![ModelMessage::system("manual request-encoding fixture").unwrap()];
    for _ in 0..8 {
        messages.push(ModelMessage::user(text.clone()).unwrap());
        messages
            .push(ModelMessage::assistant(vec![AssistantPart::Text(text.clone().into())]).unwrap());
    }
    let input = ModelRequest::new(
        messages,
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    let descriptor = ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([ReasoningPreference::Auto]),
        true,
    )
    .unwrap();

    // Warm-up is excluded from latency. RSS includes the fixture and warm-up.
    drop(request::encode("fixture-model", &descriptor, &input).unwrap());
    let started = Instant::now();
    let mut total_bytes = 0usize;
    for _ in 0..iterations {
        let bytes = request::encode(
            black_box("fixture-model"),
            black_box(&descriptor),
            black_box(&input),
        )
        .unwrap();
        total_bytes += black_box(bytes).len();
    }
    let elapsed_ns = started.elapsed().as_nanos();
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let peak_rss_kib: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();

    // Validate complete content after the timed window and HWM observation.
    let encoded = request::encode("fixture-model", &descriptor, &input).unwrap();
    let body: Value = serde_json::from_slice(&encoded).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 17);
    assert_eq!(messages[0]["content"], "manual request-encoding fixture");
    for (index, message) in messages.iter().enumerate().skip(1) {
        assert_eq!(
            message["role"],
            if index % 2 == 1 { "user" } else { "assistant" }
        );
        assert_eq!(message["content"], text);
    }
    assert_eq!(body["model"], "fixture-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["n"], 1);
    assert!(body.get("tools").is_none());
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(total_bytes, encoded.len() * iterations);
    println!(
        "{}",
        json!({
            "probe": "provider_request_encoding",
            "message_payload_bytes": text.len(),
            "message_count": messages.len(),
            "iterations": iterations,
            "elapsed_ns": elapsed_ns,
            "encoded_bytes": encoded.len(),
            "peak_rss_kib_before_validation": peak_rss_kib
        })
    );
}
