//! Copy unchanged to host-provider/examples/task108_provider_assembly.rs.
//! The actual SSE/parser/decoder source and actual Runtime ModelDriver are used.
//! Only the byte source is a deterministic fixture; no HTTP or live model runs.
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;
#[path = "../src/sse.rs"]
mod sse;

#[allow(dead_code)]
mod provider_stream {
    include!("../src/stream.rs");

    // This is only a constructor for the existing private state. No production
    // decoding, assembly, protocol check, or next() implementation is replaced.
    pub(super) fn fixture(
        body: Bytes,
        context: minicore_host_runtime::model::ModelCallContext,
    ) -> State {
        const CHUNK_BYTES: usize = 8192;
        let chunks = futures_util::stream::unfold((body, 0), |(body, offset)| async move {
            if offset == body.len() {
                None
            } else {
                let end = (offset + CHUNK_BYTES).min(body.len());
                Some((Ok(body.slice(offset..end)), (body, end)))
            }
        });
        State {
            bytes: Box::pin(chunks),
            status: 200,
            retry_at: None,
            chunk: Bytes::new(),
            offset: 0,
            parser: Parser::new(),
            pending: VecDeque::new(),
            pending_error: None,
            cancellation: context.cancellation,
            deadline: Instant::from_std(context.deadline),
            done: false,
            seen: false,
            has_output: false,
            tools: Vec::new(),
            finish: None,
            usage: None,
            raw_usage: None,
            response_id: None,
        }
    }
}

use bytes::Bytes;
use minicore_host_runtime::{
    LoopId, ToolCallId,
    model::{
        AssistantPart, Model, ModelCallContext, ModelCallEvent, ModelCallParams, ModelDescriptor,
        ModelDriver, ModelDriverConfig, ModelFinishReason, ModelLimits, ModelMessage, ModelRequest,
        ModelResponse, ModelStartFuture, ModelStream, ProviderReplay, ReasoningPreference,
        ToolCall, Usage,
    },
    tools::ToolSpec,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, hint::black_box, time::Duration};
use tokio_util::sync::CancellationToken;

const WARMUP_ITERATIONS: usize = 1;

struct FixtureModel {
    descriptor: ModelDescriptor,
    request: ModelRequest,
    body: Bytes,
}

impl Model for FixtureModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        assert_eq!(
            request, self.request,
            "fixture received a different request"
        );
        let state = provider_stream::fixture(self.body.clone(), context);
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::unfold(state, provider_stream::next)) as ModelStream)
        })
    }
}

struct Fixture {
    model: FixtureModel,
    expected: ModelResponse,
    text_fragment: String,
    expected_text_events: usize,
    frames: usize,
    arguments_bytes: usize,
    arguments_fingerprint: u64,
}

// An inexpensive deterministic identity for the generated input, not a
// cryptographic digest. The build records SHA-256 for the fixture and binary.
fn fingerprint(bytes: &[u8]) -> u64 {
    fingerprint_append(0xcbf2_9ce4_8422_2325, bytes)
}

fn fingerprint_append(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn frame(body: &mut Vec<u8>, delta: Value, finish: Option<&str>) {
    body.extend_from_slice(b"data: ");
    serde_json::to_writer(
        &mut *body,
        &json!({
            "id": "task108-response",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]
        }),
    )
    .unwrap();
    body.extend_from_slice(b"\n\n");
}

fn tool_id(index: usize) -> ToolCallId {
    format!("task108_call_{index:04}").parse().unwrap()
}

fn fixture(mode: &str, count: usize, payload_bytes: usize, fragment_bytes: usize) -> Fixture {
    let mut body = Vec::new();
    let mut parts = Vec::new();
    let mut frames = 0;
    let mut arguments_bytes = 0;
    let mut arguments_fingerprint = 0u64;
    let mut text_fragment = String::new();
    let mut expected_text_events = 0;
    let mut tools = Vec::new();
    frame(&mut body, json!({"role": "assistant"}), None);
    frames += 1;
    let (finish, reason) = match mode {
        "text" => {
            text_fragment = "abcdef0123456789".repeat(payload_bytes.div_ceil(16));
            text_fragment.truncate(payload_bytes);
            for _ in 0..count {
                frame(&mut body, json!({"content": text_fragment}), None);
            }
            frames += count;
            expected_text_events = count;
            parts.push(AssistantPart::Text(text_fragment.repeat(count).into()));
            ("stop", ModelFinishReason::Stop)
        }
        "tools" => {
            assert!(fragment_bytes > 0);
            tools.push(
                ToolSpec::new(
                    "fixture_tool".parse().unwrap(),
                    "A deterministic argument fixture; never executed",
                    json!({"type": "object"}),
                )
                .unwrap(),
            );
            let mut values = Vec::with_capacity(count);
            let mut prefixes = Vec::with_capacity(count);
            // Keep the payload only in the expected JSON value during fixture
            // construction. Slice prefix/payload/suffix into fragments instead
            // of also retaining every full serialized argument string.
            let suffix = r#"","escaped":"quote\" slash\\ newline\n tab\t","tail":[true,null,13]}"#;
            for index in 0..count {
                let pattern = format!("tool{index:04}:abcdef");
                let mut payload = pattern.repeat(payload_bytes.div_ceil(pattern.len()));
                payload.truncate(payload_bytes);
                let value = json!({
                    "index": index, "payload": payload,
                    "escaped": "quote\" slash\\ newline\n tab\t",
                    "tail": [true, null, 13]
                });
                let prefix = format!("{{\"index\":{index},\"payload\":\"");
                let payload = value["payload"].as_str().unwrap();
                assert!(
                    payload.is_ascii(),
                    "fragment offsets must be UTF-8 boundaries"
                );
                arguments_bytes += prefix.len() + payload.len() + suffix.len();
                let hash = [prefix.as_str(), payload, suffix]
                    .into_iter()
                    .fold(0xcbf2_9ce4_8422_2325, |hash, part| {
                        fingerprint_append(hash, part.as_bytes())
                    });
                arguments_fingerprint =
                    arguments_fingerprint.wrapping_add(hash.rotate_left((index % 64) as u32));
                prefixes.push(prefix);
                values.push(value);
            }
            let longest =
                prefixes.iter().map(String::len).max().unwrap() + payload_bytes + suffix.len();
            // Round-robin fragments keep all calls open together. Each call has
            // different bytes, so misrouting/interleaving cannot pass equality.
            for offset in (0..longest).step_by(fragment_bytes) {
                for (index, value) in values.iter().enumerate() {
                    let segments = [
                        prefixes[index].as_str(),
                        value["payload"].as_str().unwrap(),
                        suffix,
                    ];
                    let raw_len: usize = segments.iter().map(|part| part.len()).sum();
                    if offset >= raw_len {
                        continue;
                    }
                    let end = (offset + fragment_bytes).min(raw_len);
                    let mut fragment = String::with_capacity(end - offset);
                    let mut position = 0;
                    for segment in segments {
                        let start = offset.saturating_sub(position).min(segment.len());
                        let stop = end.saturating_sub(position).min(segment.len());
                        fragment.push_str(&segment[start..stop]);
                        position += segment.len();
                    }
                    let tool = if offset == 0 {
                        json!({"index": index, "id": tool_id(index).as_str(), "type": "function",
                            "function": {"name": "fixture_tool", "arguments": fragment}})
                    } else {
                        json!({"index": index, "function": {"arguments": fragment}})
                    };
                    frame(&mut body, json!({"tool_calls": [tool]}), None);
                    frames += 1;
                }
            }
            for (index, value) in values.into_iter().enumerate() {
                parts.push(AssistantPart::ToolCall(
                    ToolCall::new(
                        tool_id(index),
                        "fixture_tool".parse().unwrap(),
                        value,
                        index.try_into().unwrap(),
                    )
                    .unwrap(),
                ));
            }
            ("tool_calls", ModelFinishReason::ToolCalls)
        }
        _ => panic!("mode must be text or tools"),
    };
    frame(&mut body, json!({}), Some(finish));
    let provider_usage = json!({
        "prompt_tokens": 100, "completion_tokens": 80, "total_tokens": 180,
        "prompt_tokens_details": {"cached_tokens": 20},
        "completion_tokens_details": {"reasoning_tokens": 10}
    });
    body.extend_from_slice(b"data: ");
    serde_json::to_writer(
        &mut body,
        &json!({"id": "task108-response", "choices": [], "usage": provider_usage}),
    )
    .unwrap();
    body.extend_from_slice(b"\n\ndata: [DONE]\n\n");
    frames += 3;
    let request = ModelRequest::new(
        vec![ModelMessage::user("task108 deterministic SSE and Runtime fixture").unwrap()],
        tools,
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    let descriptor = ModelDescriptor::new(
        "fixture:task108".parse().unwrap(),
        128_000,
        BTreeSet::from([ReasoningPreference::Auto]),
        true,
    )
    .unwrap();
    let usage = Usage::new(80, 70, 10)
        .with_cache_read_tokens(Some(20))
        .with_provider_total_tokens(Some(180));
    let replay = ProviderReplay::new(
        "minicore-host.openai-chat.v1",
        json!({"finish_reason": finish, "usage": provider_usage}),
    )
    .unwrap();
    let expected =
        ModelResponse::new_with_provider_replay(parts, reason, usage, Some(replay)).unwrap();
    Fixture {
        model: FixtureModel {
            descriptor,
            request,
            body: body.into(),
        },
        expected,
        text_fragment,
        expected_text_events,
        frames,
        arguments_bytes,
        arguments_fingerprint,
    }
}

fn memory() -> Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let field = |name: &str| -> u64 {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    json!({"rss_kib": field("VmRSS:"), "peak_rss_kib": field("VmHWM:")})
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        6,
        "mode count payload_bytes fragment_bytes iterations"
    );
    let mode = &args[1];
    let count: usize = args[2].parse().unwrap();
    let payload_bytes: usize = args[3].parse().unwrap();
    let fragment_bytes: usize = args[4].parse().unwrap();
    let iterations: usize = args[5].parse().unwrap();
    assert!(count > 0 && payload_bytes > 0 && iterations > 0);
    let fixture = fixture(mode, count, payload_bytes, fragment_bytes);
    let wire_fingerprint = fingerprint(&fixture.model.body);
    let request_bytes = serde_json::to_vec(&fixture.model.request).unwrap();
    let request_fingerprint = fingerprint(&request_bytes);
    let driver = ModelDriver::new(
        &fixture.model,
        ModelDriverConfig::new(Duration::from_secs(120), 1, Duration::from_millis(1)),
    )
    .unwrap();
    let loop_id: LoopId = "lup_00000000000000000000000000000108".parse().unwrap();
    let after_setup = memory();
    let mut after_warmup = Value::Null;
    let mut timed_ns = Vec::with_capacity(iterations);
    let mut warmup_ns = Vec::with_capacity(WARMUP_ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + iterations) {
        let params = ModelCallParams {
            loop_id,
            request_index: 0,
            cancellation: CancellationToken::new(),
            deadline: None,
        };
        let request = fixture.model.request.clone();
        let mut attempts = 0;
        let mut text_events = 0;
        let mut progress_matches = true;
        let mut observer = |event| match event {
            ModelCallEvent::AttemptStarted(attempt) => attempts = attempt,
            ModelCallEvent::TextDelta(delta) => {
                text_events += 1;
                progress_matches &= delta.as_str() == fixture.text_fragment;
            }
            ModelCallEvent::ReasoningDelta(_) | ModelCallEvent::RetryWaiting(_) => {
                progress_matches = false;
            }
        };
        let started = std::time::Instant::now();
        let report = black_box(driver.call(black_box(request), params, &mut observer).await);
        let elapsed = started.elapsed().as_nanos();
        if iteration < WARMUP_ITERATIONS {
            warmup_ns.push(elapsed);
        } else {
            timed_ns.push(elapsed);
        }
        // Full value equality covers text, each exact JSON argument, identity,
        // call/part order, finish, usage and replay. It allocates no result copy
        // and is deliberately outside the per-call timer.
        assert_eq!(report.attempts, 1);
        assert_eq!(attempts, 1);
        assert_eq!(report.retry_wait_elapsed, Duration::ZERO);
        assert!(progress_matches);
        assert_eq!(text_events, fixture.expected_text_events);
        assert_eq!(report.outcome.unwrap(), fixture.expected);
        if iteration + 1 == WARMUP_ITERATIONS {
            after_warmup = memory();
        }
    }
    let after_measurement = memory();
    // A canonical expected-output fingerprint is computed only after memory
    // observation; serialization must not create the reported process peak.
    let expected_bytes = serde_json::to_vec(&fixture.expected).unwrap();
    println!(
        "{}",
        json!({
            "probe": "provider_sse_runtime_assembly", "mode": mode,
            "count": count, "payload_bytes": payload_bytes, "fragment_bytes": fragment_bytes,
            "chunk_bytes": 8192, "iterations": iterations,
            "warmup_iterations": WARMUP_ITERATIONS, "warmup_elapsed_ns": warmup_ns,
            "iteration_elapsed_ns": timed_ns, "elapsed_ns": timed_ns.iter().sum::<u128>(),
            "sse_frames": fixture.frames, "sse_bytes": fixture.model.body.len(),
            "sse_fnv1a64": format!("{wire_fingerprint:016x}"),
            "request_bytes": request_bytes.len(), "request_fnv1a64": format!("{request_fingerprint:016x}"),
            "arguments_bytes": fixture.arguments_bytes,
            "arguments_fingerprint": format!("{:016x}", fixture.arguments_fingerprint),
            "expected_response_bytes": expected_bytes.len(),
            "expected_response_fnv1a64": format!("{:016x}", fingerprint(&expected_bytes)),
            "memory_after_setup": after_setup, "memory_after_warmup": after_warmup,
            "memory_after_measurement": after_measurement,
            "all_exact_request_response_and_progress_checks_passed": true,
            "memory_scope": "Linux process VmRSS/VmHWM; includes fixture, expected response, runtime, warmup and allocator retention; excludes final output serialization"
        })
    );
}
