//! Task104 manual timing probe, copied temporarily to host-compaction/examples.
//! Usage: task104_compaction [historical_groups] [body_bytes] [repetitions]
//! This uses public prepare/generate and a deterministic Model fixture. It does
//! not measure a live provider, storage, Session owners, or end-to-end latency.
use minicore_host_compaction::{HistorySnapshot, generate, prepare};
use minicore_host_protocol::CompactionSettings;
use minicore_runtime::{
    HistoryItem, LoopId, UserHistory, UserInput, UserMessageKind,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelLimits,
        ModelMessage, ModelRequest, ModelStartFuture, ModelStream, ReasoningPreference,
    },
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    error::Error,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct DeterministicSummaryFixture {
    descriptor: ModelDescriptor,
    calls: AtomicU64,
}

impl Model for DeterministicSummaryFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {
            Ok(Box::pin(futures_util::stream::iter([
                Ok(ModelEvent::text_delta(
                    "Preserve the historical facts and current task.",
                )),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}

fn argument(index: usize, fallback: usize) -> Result<usize, Box<dyn Error>> {
    match std::env::args().nth(index) {
        Some(value) => {
            let number = value.parse::<usize>()?;
            if number == 0 {
                return Err("probe arguments must be positive".into());
            }
            Ok(number)
        }
        None => Ok(fallback),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let groups = argument(1, 256)?;
    let body_bytes = argument(2, 1024)?;
    let repetitions = argument(3, 5)?;
    let pattern = "Historical fact with \"quotes\", \\path, and newline.\n";
    let mut body = pattern.repeat(body_bytes.div_ceil(pattern.len()));
    body.truncate(body_bytes); // ASCII fixture data makes every byte a boundary.
    let loop_id = LoopId::new()?;
    let mut items = Vec::with_capacity(groups.saturating_add(1));
    let mut messages = Vec::with_capacity(groups.saturating_add(1));
    let mut source_text_bytes = 0_u64;
    for index in 0..groups {
        let text = format!("source-{index:08}: {body}");
        source_text_bytes = source_text_bytes.saturating_add(text.len() as u64);
        items.push(HistoryItem::User(UserHistory {
            loop_id,
            kind: UserMessageKind::Prompt,
            input: UserInput::text(&text)?,
        }));
        messages.push(ModelMessage::user(text)?);
    }
    let current = "Protected current task";
    items.push(HistoryItem::User(UserHistory {
        loop_id,
        kind: UserMessageKind::Prompt,
        input: UserInput::text(current)?,
    }));
    messages.push(ModelMessage::user(current)?);
    let source = HistorySnapshot {
        revision: groups as u64,
        items: items.into(),
    };
    let normal = ModelRequest::new(
        messages,
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )?;
    let fixture = DeterministicSummaryFixture {
        descriptor: ModelDescriptor::new(
            "fixture/task104-summary".parse()?,
            source_text_bytes.saturating_mul(2).saturating_add(4096),
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )?,
        calls: AtomicU64::new(0),
    };
    // Manual compaction below the automatic trigger is permitted. This soft
    // target retains a small tail and exercises a large whole-exchange chunk.
    let settings = CompactionSettings {
        enabled: true,
        trigger_percent: 95,
        target_percent: 1,
    };
    for repetition in 0..repetitions {
        fixture.calls.store(0, Ordering::Relaxed);
        let started = Instant::now();
        let plan = prepare(
            source.clone(),
            groups,
            fixture.descriptor(),
            &settings,
            normal.clone(),
        )?
        .ok_or("probe has no compactable prefix")?;
        let prepared = Instant::now();
        let candidate = generate(
            &plan,
            &fixture,
            ModelCallContext::new(
                loop_id,
                0,
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(600),
            ),
        )
        .await?;
        let finished = Instant::now();
        let result = json!({
            "fixture": "DeterministicSummaryFixture",
            "historical_groups": groups,
            "body_bytes": body_bytes,
            "source_text_bytes": source_text_bytes,
            "repetition": repetition,
            "covered_items": candidate.covered().end,
            "utility_calls": fixture.calls.load(Ordering::Relaxed),
            "before_tokens": candidate.before_tokens,
            "after_tokens": candidate.after_tokens,
            "prepare_ns": (prepared - started).as_nanos(),
            "generate_ns": (finished - prepared).as_nanos(),
            "total_ns": (finished - started).as_nanos(),
        });
        std::hint::black_box(candidate);
        println!("{result}");
    }
    Ok(())
}
