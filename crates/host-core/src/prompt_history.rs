use minicore_runtime::{
    HistoryItem,
    model::{AssistantPart, ModelMessage, ToolCall},
    prompt::{DefaultPromptProvider, PreparedPrompt, PromptError, PromptProvider, PromptRequest},
};
use std::collections::BTreeSet;

/// Prepare native history for one model request, without changing Session or
/// persisted history. Only unclosed calls in the host's *base* history become
/// ordinary historical text. The active loop's appended exchanges stay strict.
///
/// Complete exchanges use Runtime's default projection unchanged. A mixed
/// response keeps its parts in order and its complete calls/results associated
/// by their upstream IDs; only the retained calls' request-local indices become
/// dense again, as required by `ModelMessage`.
///
/// Provider replay remains opaque and is never exposed as text or used to
/// invent signatures. The actual model adapter must still validate it. Host's
/// OpenAI Chat adapter replays its own finish/usage metadata through canonical
/// parts and explicitly rejects other replay formats and signed reasoning.
pub async fn prepare_prompt_history(
    request: PromptRequest<'_>,
) -> Result<PreparedPrompt, PromptError> {
    let base = request.history.base();
    let mut prompt = DefaultPromptProvider::new(None).prepare(request).await?;
    for (position, item) in base.iter().enumerate() {
        let HistoryItem::Assistant(assistant) = item else {
            continue;
        };
        // An exchange ends at the next non-result message. Never match a later
        // Turn's reused provider ID, or close a call with another loop/request.
        let completed: BTreeSet<_> = base[position + 1..]
            .iter()
            .map_while(|item| match item {
                HistoryItem::ToolResult(result) => Some(result),
                _ => None,
            })
            .filter(|result| {
                result.loop_id == assistant.loop_id
                    && result.request_index == assistant.request_index
            })
            .map(|result| (&result.call_id, &result.tool_name))
            .collect();
        let is_complete = |call: &ToolCall| completed.contains(&(call.tool_call_id(), call.name()));
        if assistant
            .content
            .iter()
            .filter_map(AssistantPart::as_tool_call)
            .all(is_complete)
        {
            continue;
        }
        let mut parts = assistant.content.clone();
        let mut next_index = 0;
        for part in &mut parts {
            let AssistantPart::ToolCall(call) = part else {
                continue;
            };
            if is_complete(call) {
                // This index is not the upstream ToolCallId and is not emitted
                // in OpenAI Chat's wire request. Native history stays untouched.
                *call = ToolCall::new(
                    call.tool_call_id().clone(),
                    call.name().clone(),
                    call.arguments().clone(),
                    next_index,
                )
                .map_err(|_| PromptError::InvalidHistory)?;
                next_index += 1;
            } else {
                let record = serde_json::json!({
                    "loop_id": assistant.loop_id,
                    "request_index": assistant.request_index,
                    "tool_call": call,
                });
                *part = AssistantPart::Text(
                    format!(
                        "\nRecorded historical tool call (not a new execution request). \
                     The current history has no corresponding result record; \
                     this does not establish whether the tool ran or its outcome.\n{record}\n"
                    )
                    .into(),
                );
            }
        }
        prompt.messages[position] =
            ModelMessage::assistant_with_provider_replay(parts, assistant.provider_replay.clone())
                .map_err(|_| PromptError::InvalidHistory)?;
    }
    Ok(prompt)
}
