use super::*;
use crate::{ContextEstimate, HistorySnapshot};
use minicore_host_runtime::{
    AssistantHistory, HistoryItem, LoopId, ToolResultHistory,
    model::{
        ModelDescriptor, ModelEvent, ModelLimits, ModelStartFuture, ModelStream,
        ReasoningPreference, ToolCall,
    },
    tools::{ToolOutput, ToolResultOutcome},
};
use std::{collections::BTreeSet, sync::Mutex, time::Duration};
use tokio_util::sync::CancellationToken;

fn plan(reasoning: ReasoningPreference, limits: ModelLimits) -> CompactionPlan {
    let items = (0..9)
        .map(|index| {
            HistoryItem::Summary(SummaryHistory {
                content: Text::new(format!(
                    "source-{index}: {}",
                    "fact with quotes \" and slash \\ and Unicode 原文🐈. ".repeat(30)
                )),
            })
        })
        .collect::<Vec<_>>();
    let messages = items
        .iter()
        .map(|item| {
            let HistoryItem::Summary(summary) = item else {
                unreachable!()
            };
            crate::plan::summary_message(summary).unwrap()
        })
        .collect();
    let original = ModelRequest::new(messages, vec![], limits, reasoning).unwrap();
    let input_tokens = estimate_request_tokens(&original).unwrap();
    CompactionPlan {
        source: HistorySnapshot {
            revision: 3,
            items: items.into(),
        },
        covered_end: 8,
        groups: (0..8).map(|index| index..index + 1).collect(),
        descriptor: descriptor(reasoning, 4096),
        original,
        message_offset: 0,
        summary_target: 1,
        estimate: ContextEstimate {
            input_tokens,
            context_window: 4096,
            trigger_tokens: 4096,
            target_tokens: 2048,
            automatic_due: false,
        },
    }
}

fn descriptor(reasoning: ReasoningPreference, window: u64) -> ModelDescriptor {
    ModelDescriptor::new(
        "fixture/compaction-size".parse().unwrap(),
        window,
        BTreeSet::from([reasoning]),
        true,
    )
    .unwrap()
}

#[test]
fn incremental_size_matches_serialized_requests_with_escaping_and_runtime_options() {
    let records = [
        json!([{"text": "plain"}]),
        json!([{"text": "\"quoted\" \\ slash\nnewline\ttab\r\n\0\u{1f}"}]),
        json!([{"text": "原文🐈 e\u{301} \u{2028}\u{2029}", "nested": [null, true, 123]}]),
        json!([{"text": "x"}, {"text": "yz"}]),
    ];
    let previous = SummaryHistory {
        content: Text::new("Earlier \"summary\" \\ with\nUnicode 原文 and\ttab"),
    };
    for reasoning in [
        ReasoningPreference::Auto,
        ReasoningPreference::Disabled,
        ReasoningPreference::Low,
        ReasoningPreference::Medium,
        ReasoningPreference::High,
        ReasoningPreference::XHigh,
        ReasoningPreference::Max,
        ReasoningPreference::Ultra,
    ] {
        for limits in [
            ModelLimits::default(),
            ModelLimits::new(Some(4096), Some(97)).unwrap(),
        ] {
            let plan = plan(reasoning, limits);
            for previous in [None, Some(&previous)] {
                let mut bytes =
                    serialized_request_bytes(&utility_request(&plan, previous, &[]).unwrap())
                        .unwrap();
                for (index, record) in records.iter().enumerate() {
                    bytes = bytes
                        .saturating_add(utility_record_bytes(record).unwrap())
                        .saturating_add(u64::from(index > 0));
                    let request = utility_request(&plan, previous, &records[..=index]).unwrap();
                    assert_eq!(bytes, serde_json::to_vec(&request).unwrap().len() as u64);
                    assert_eq!(
                        bytes.div_ceil(4),
                        estimate_request_tokens(&request).unwrap()
                    );
                }
            }
        }
    }
}

/// A deterministic Model fixture; all requests still use Runtime's real types.
struct DeterministicSummaryFixture {
    descriptor: ModelDescriptor,
    requests: Mutex<Vec<Vec<u8>>>,
}
impl Model for DeterministicSummaryFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.requests
            .lock()
            .unwrap()
            .push(serde_json::to_vec(&request).unwrap());
        Box::pin(async {
            Ok(Box::pin(futures_util::stream::iter([
                Ok(ModelEvent::text_delta("kept\r")),
                Ok(ModelEvent::text_delta("\n\0ready")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}

fn context() -> ModelCallContext {
    ModelCallContext::new(
        LoopId::new().unwrap(),
        5,
        CancellationToken::new(),
        Instant::now() + Duration::from_secs(5),
    )
}

fn records(plan: &CompactionPlan) -> Vec<Value> {
    plan.groups
        .iter()
        .map(|group| {
            group_data(
                &plan.source.items[group.clone()],
                &plan.original.messages()[group.clone()],
            )
            .unwrap()
        })
        .collect()
}

/// Preserve the former full-request selection as an independent test oracle.
fn full_request_chunks(plan: &CompactionPlan, records: &[Value]) -> Vec<Vec<u8>> {
    let summary = SummaryHistory {
        content: Text::new("kept\n ready"),
    };
    let mut previous = None;
    let mut position = 0;
    let mut requests = Vec::new();
    let goal = plan
        .estimate
        .context_window
        .saturating_sub(plan.summary_target)
        .max(1);
    while position < records.len() {
        let mut end = position;
        while end < records.len() {
            let proposed = utility_request(plan, previous, &records[position..=end]).unwrap();
            if end > position && estimate_request_tokens(&proposed).unwrap() > goal {
                break;
            }
            end += 1;
        }
        let request = utility_request(plan, previous, &records[position..end]).unwrap();
        requests.push(serde_json::to_vec(&request).unwrap());
        previous = Some(&summary);
        position = end;
    }
    requests
}

#[tokio::test]
async fn chunk_boundaries_and_requests_match_full_estimation_including_oversized_groups() {
    let mut plan = plan(
        ReasoningPreference::High,
        // Vary the actual descriptor below without declaring a larger fixed
        // context limit, which the shared model driver correctly rejects.
        ModelLimits::new(None, Some(97)).unwrap(),
    );
    let records = records(&plan);
    let boundary =
        estimate_request_tokens(&utility_request(&plan, None, &records[..2]).unwrap()).unwrap();
    // Exact threshold, one token below it, and an indivisible oversized group.
    for goal in [boundary, boundary - 1, 1] {
        let window = goal + plan.summary_target;
        plan.estimate.context_window = window;
        plan.descriptor = descriptor(plan.original.reasoning(), window);
        let fixture = DeterministicSummaryFixture {
            descriptor: plan.descriptor.clone(),
            requests: Mutex::new(Vec::new()),
        };
        let candidate = generate(&plan, &fixture, context()).await.unwrap();
        let actual = fixture.requests.lock().unwrap();
        assert_eq!(*actual, full_request_chunks(&plan, &records));
        assert_eq!(candidate.summary.content.as_str(), "kept\n ready");
        assert_eq!(candidate.utility_usage.len(), actual.len());
        assert!(candidate.utility_usage.iter().all(|usage| usage.completed));
        if goal == 1 {
            assert_eq!(actual.len(), records.len());
        }
    }
}

#[tokio::test]
async fn invalid_next_group_is_rejected_before_finishing_an_oversized_chunk() {
    let mut plan = plan(ReasoningPreference::Auto, ModelLimits::default());
    let loop_id = LoopId::new().unwrap();
    let call = ToolCall::new(
        "control-call".parse().unwrap(),
        "read".parse().unwrap(),
        json!({"path": "invalid historical control \u{85}"}),
        0,
    )
    .unwrap();
    let mut items = plan.source.items.to_vec();
    // This is valid native tool JSON. Unlike C0 controls, serde_json leaves its
    // Unicode control unescaped when it becomes historical user-message text.
    items.splice(
        1..2,
        [
            HistoryItem::Assistant(AssistantHistory {
                loop_id,
                request_index: 0,
                model: plan.descriptor.model_ref.clone(),
                reasoning: ReasoningPreference::Auto,
                content: vec![AssistantPart::ToolCall(call.clone())],
                provider_replay: None,
                finish_reason: ModelFinishReason::ToolCalls,
                usage: Usage::default(),
            }),
            HistoryItem::ToolResult(ToolResultHistory {
                loop_id,
                request_index: 0,
                call_id: call.tool_call_id().clone(),
                tool_name: call.name().clone(),
                outcome: ToolResultOutcome::Success,
                output: ToolOutput::new("historical output").unwrap(),
            }),
        ],
    );
    let mut messages = plan.original.messages().to_vec();
    messages.splice(
        1..2,
        [
            ModelMessage::assistant(vec![AssistantPart::ToolCall(call.clone())]).unwrap(),
            ModelMessage::tool_with_outcome(
                call.tool_call_id().clone(),
                ToolOutput::new("historical output").unwrap(),
                ToolResultOutcome::Success,
            )
            .unwrap(),
        ],
    );
    plan.original = ModelRequest::new(
        messages,
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    plan.covered_end += 1;
    plan.groups = std::iter::once(0..1)
        .chain(std::iter::once(1..3))
        .chain((3..plan.covered_end).map(|index| index..index + 1))
        .collect();
    plan.source.items = items.into();
    plan.estimate.context_window = 2;
    plan.descriptor = descriptor(ReasoningPreference::Auto, 2);
    let fixture = DeterministicSummaryFixture {
        descriptor: plan.descriptor.clone(),
        requests: Mutex::new(Vec::new()),
    };
    let source = records(&plan);
    assert!(matches!(
        utility_request(&plan, None, &source[..2]),
        Err(CompactionError::Serialization)
    ));
    let failure = generate(&plan, &fixture, context()).await.unwrap_err();
    assert_eq!(failure.error, CompactionError::Serialization);
    assert!(failure.utility_usage.is_empty());
    assert!(fixture.requests.lock().unwrap().is_empty());
}
