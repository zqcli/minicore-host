//! Public-API serialization probe, not a test of live model or transport speed.
//!
//! Copy this identical file to crates/host-protocol/examples/task104_protocol.rs
//! in both source checkouts and build with the same locked release environment.
//! Run the executable with an optional positive target encoded bytes per case
//! (default 64 MiB). Keep that argument identical for baseline and candidate.
//! Iteration counts are fixture controls, not product resource limits.
use minicore_host_protocol::{
    AgentConfig, OutputDelta, Role, SessionConfig, SessionDefaults, TextRangeData, ToolBinding,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{hint::black_box, time::Instant};

fn measure<T>(case: &str, value: &T, target_bytes: usize)
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    let expected = serde_json::to_vec(value).expect("valid probe value");
    let decoded: T = serde_json::from_slice(&expected).expect("valid probe wire shape");
    assert!(decoded == *value, "probe roundtrip changed {case}");
    let iterations = (target_bytes / expected.len()).clamp(16, 20_000);
    for _ in 0..iterations.min(64) {
        black_box(serde_json::to_vec(black_box(value)).unwrap());
    }

    let started = Instant::now();
    let mut total_bytes = 0_u64;
    for _ in 0..iterations {
        let bytes = serde_json::to_vec(black_box(value)).expect("probe serialization");
        total_bytes += black_box(bytes.len()) as u64;
        black_box(bytes);
    }
    let elapsed_ns = u64::try_from(started.elapsed().as_nanos()).unwrap();
    assert_eq!(total_bytes, expected.len() as u64 * iterations as u64);
    assert_eq!(serde_json::to_vec(value).unwrap(), expected);
    println!(
        "{}",
        serde_json::json!({
            "schema": "task104.protocol.v1",
            "case": case,
            "encoded_bytes": expected.len(),
            "target_bytes": target_bytes,
            "iterations": iterations,
            "total_bytes": total_bytes,
            "elapsed_ns": elapsed_ns,
            "ns_per_iteration": elapsed_ns as f64 / iterations as f64,
        })
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let target_bytes = args
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .expect("target bytes must be an integer")
        })
        .unwrap_or(64 * 1024 * 1024);
    assert!(target_bytes > 0, "target bytes must be positive");
    assert!(args.next().is_none(), "expected at most one argument");

    for size in [64, 4 * 1024, 64 * 1024, 1024 * 1024] {
        let text = "a中🙂".repeat(size / 8);
        assert_eq!(text.len(), size);
        let delta = OutputDelta {
            channel: "text".into(),
            start: 7,
            end: 7 + size as u64,
            text: text.clone(),
        };
        measure(&format!("output_delta_{size}"), &delta, target_bytes);
        let range = TextRangeData {
            channel: delta.channel,
            start: delta.start,
            end: delta.end,
            text: delta.text,
            has_more: true,
        };
        measure(&format!("text_range_{size}"), &range, target_bytes);

        for count in [0, 8, 64] {
            let config = SessionConfig {
                role: Role {
                    instructions: text.clone(),
                    ..Role::default()
                },
                tools: (0..count)
                    .map(|index| ToolBinding {
                        name: format!("tool_{index}"),
                    })
                    .collect(),
                ..SessionConfig::default()
            };
            measure(
                &format!("session_config_{size}_{count}_tools"),
                &config,
                target_bytes,
            );
            let agent = AgentConfig {
                role: config.role,
                session_defaults: SessionDefaults {
                    model: config.model,
                    reasoning: config.reasoning,
                    tools: config.tools,
                    workspace: config.workspace,
                    execution: config.execution,
                },
            };
            measure(
                &format!("agent_config_{size}_{count}_tools"),
                &agent,
                target_bytes,
            );
        }
    }
}
