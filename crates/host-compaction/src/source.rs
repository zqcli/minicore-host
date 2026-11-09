use crate::CompactionError;
use minicore_runtime::{HistoryItem, model::AssistantPart};
use serde_json::{Value, json};
use std::{collections::BTreeSet, ops::Range};

/// Keep an assistant and all its adjacent results together, including an old
/// exchange with genuinely missing results. Never match a later reused ID.
pub(crate) fn exchange_ranges(items: &[HistoryItem]) -> Result<Vec<Range<usize>>, CompactionError> {
    let mut groups = Vec::new();
    let mut position = 0;
    while position < items.len() {
        let start = position;
        position += 1;
        match &items[start] {
            HistoryItem::Assistant(assistant) => {
                let calls: BTreeSet<_> = assistant
                    .content
                    .iter()
                    .filter_map(AssistantPart::as_tool_call)
                    .map(|call| (call.tool_call_id(), call.name()))
                    .collect();
                let mut call_ids = BTreeSet::new();
                if calls.len()
                    != assistant
                        .content
                        .iter()
                        .filter_map(AssistantPart::as_tool_call)
                        .count()
                    || calls.iter().any(|(id, _)| !call_ids.insert(*id))
                {
                    return Err(CompactionError::InvalidHistory);
                }
                let mut seen = BTreeSet::new();
                while let Some(HistoryItem::ToolResult(result)) = items.get(position) {
                    let key = (&result.call_id, &result.tool_name);
                    if result.loop_id != assistant.loop_id
                        || result.request_index != assistant.request_index
                        || !calls.contains(&key)
                        || !seen.insert(key)
                    {
                        return Err(CompactionError::InvalidHistory);
                    }
                    position += 1;
                }
            }
            HistoryItem::ToolResult(_) => return Err(CompactionError::InvalidHistory),
            HistoryItem::User(_) | HistoryItem::Summary(_) => {}
        }
        groups.push(start..position);
    }
    Ok(groups)
}

/// Summarization input is historical data, never executable assistant/tool
/// messages. Preserve full text, arguments, results, identities and outcomes.
/// Opaque provider replay and encrypted/signature artifacts are not prose and
/// are not decoded or exposed as text. Their presence is explicit instead.
pub(crate) fn group_data(items: &[HistoryItem]) -> Value {
    let records: Vec<_> = items
        .iter()
        .map(|item| match item {
            HistoryItem::Assistant(value) => {
                let parts: Vec<_> = value.content.iter().map(|part| match part {
                    AssistantPart::Text(text) => json!({"type": "text", "text": text}),
                    AssistantPart::ToolCall(call) => {
                        let has_result = items.iter().any(|item| matches!(item,
                            HistoryItem::ToolResult(result)
                            if result.loop_id == value.loop_id
                                && result.request_index == value.request_index
                                && &result.call_id == call.tool_call_id()
                                && &result.tool_name == call.name()));
                        json!({"type": "tool_call", "call": call,
                            "result_record": if has_result { "present" } else { "missing; execution and outcome are unknown" }})
                    }
                    AssistantPart::Reasoning(value) => json!({
                        "type": "reasoning", "text": value.text(), "summary": value.summary(),
                        "opaque_artifacts_present": value.encrypted().is_some() || value.signature().is_some(),
                    }),
                }).collect();
                json!({"type": "assistant", "loop_id": value.loop_id,
                    "request_index": value.request_index, "model": value.model,
                    "parts": parts, "finish_reason": value.finish_reason,
                    "opaque_provider_metadata_present": value.provider_replay.is_some()})
            }
            // These Runtime variants contain no opaque provider replay. Their
            // native serialization retains the actual outcomes and provenance.
            item => json!(item),
        })
        .collect();
    Value::Array(records)
}
