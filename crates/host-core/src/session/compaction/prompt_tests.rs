use super::incomplete_tail;
use minicore_host_runtime::{
    AssistantHistory, HistoryItem, LoopId, SummaryHistory, Text, ToolResultHistory,
    model::{AssistantPart, ModelFinishReason, ReasoningPreference, ToolCall, Usage},
    tools::{ToolOutput, ToolResultOutcome},
};

fn exchange() -> Vec<HistoryItem> {
    let loop_id = LoopId::new().unwrap();
    let calls = (0..64)
        .map(|index| {
            ToolCall::new(
                format!("call-{index}").parse().unwrap(),
                "read".parse().unwrap(),
                serde_json::json!({"index": index}),
                index,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let mut items = vec![HistoryItem::Assistant(AssistantHistory {
        loop_id,
        request_index: 3,
        model: "fixture/model".parse().unwrap(),
        reasoning: ReasoningPreference::Auto,
        content: calls.iter().cloned().map(AssistantPart::ToolCall).collect(),
        provider_replay: None,
        finish_reason: ModelFinishReason::ToolCalls,
        usage: Usage::default(),
    })];
    // Results need not arrive in call order. Every terminal outcome counts as
    // a recorded result, including failure and cancellation.
    items.extend(calls.iter().rev().map(|call| {
        HistoryItem::ToolResult(ToolResultHistory {
            loop_id,
            request_index: 3,
            call_id: call.tool_call_id().clone(),
            tool_name: call.name().clone(),
            outcome: if call.call_index().is_multiple_of(2) {
                ToolResultOutcome::Failed
            } else {
                ToolResultOutcome::Cancelled
            },
            output: ToolOutput::new("recorded outcome").unwrap(),
        })
    }));
    items
}

#[test]
fn incomplete_tail_matches_full_result_identity_for_every_call() {
    let items = exchange();
    assert_eq!(incomplete_tail(&items), None);
    assert_eq!(incomplete_tail(&items[..items.len() - 1]), Some(0));
    for mismatch in 0..4 {
        let mut items = items.clone();
        let HistoryItem::ToolResult(result) = &mut items[1] else {
            unreachable!()
        };
        match mismatch {
            0 => result.loop_id = LoopId::new().unwrap(),
            1 => result.request_index += 1,
            2 => result.call_id = "another-call".parse().unwrap(),
            3 => result.tool_name = "another_tool".parse().unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(incomplete_tail(&items), Some(0));
    }
}

#[test]
fn incomplete_tail_does_not_use_earlier_results_for_reused_calls() {
    let mut items = exchange();
    let new_assistant = items[0].clone();
    let start = items.len();
    items.push(new_assistant);
    assert_eq!(incomplete_tail(&items), Some(start));
    items.push(HistoryItem::Summary(SummaryHistory {
        content: Text::new("later boundary"),
    }));
    assert_eq!(incomplete_tail(&items), None);
    assert_eq!(incomplete_tail(&[]), None);
}
