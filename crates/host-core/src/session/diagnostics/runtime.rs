//! Actual Runtime retry boundaries and returned request facts only.
use super::*;
use minicore_runtime::model::{ModelFinishReason, ModelRetryProgress, RetryHint, Usage};

pub(in crate::session) fn model_retry(
    source: &TurnIdentity,
    loop_id: LoopId,
    request_index: u32,
    base: u64,
    retry: &ModelRetryProgress,
) -> Record {
    let mut data = EventData::default()
        .context(turn_context(source))
        .fault(&runtime_model_fault(&retry.error))
        .attribute("loop_id", loop_id.to_string())
        .attribute("request_index", u64::from(request_index))
        .attribute("attempt", u64::from(retry.attempt))
        .attribute("max_attempts", u64::from(retry.max_attempts));
    if let Some(index) = base.checked_add(u64::from(request_index)) {
        data = data.attribute("response_index", index);
    }
    if let RetryHint::Retryable {
        retry_after: Some(delay),
    } = retry.error.retry_hint()
        && let Ok(delay) = u64::try_from(delay.as_millis())
    {
        data = data.attribute("retry_after_ms", delay);
    }
    Record::new(
        event(
            if retry.attempt == 2 {
                Level::WARN
            } else {
                Level::DEBUG
            },
            Scope::Runtime,
            "session.runtime",
            "model.retry_scheduled",
            "Runtime scheduled its next actual model attempt",
            &[
                "loop_id",
                "request_index",
                "response_index",
                "attempt",
                "max_attempts",
                "retry_after_ms",
            ],
        ),
        data,
    )
}

const MODEL_ATTRIBUTES: &[&str] = &[
    "loop_id",
    "request_index",
    "response_index",
    "attempt",
    "outcome",
    "finish_reason",
    "input_tokens",
    "output_tokens",
    "reasoning_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "provider_total_tokens",
    "wait_duration_ms",
];
fn usage(mut data: EventData, usage: Option<Usage>) -> EventData {
    if let Some(usage) = usage {
        for (key, value) in [
            ("input_tokens", usage.input_tokens()),
            ("output_tokens", usage.output_tokens()),
            ("reasoning_tokens", usage.reasoning_tokens()),
            ("cache_read_tokens", usage.cache_read_tokens()),
            ("cache_write_tokens", usage.cache_write_tokens()),
            ("provider_total_tokens", usage.provider_total_tokens()),
        ] {
            if let Some(value) = value {
                data = data.attribute(key, value);
            }
        }
    }
    data
}

/// Called once for the actual joined report, before any subsequent save wait.
/// No bodies are copied or formatted, and no observer timestamp becomes a
/// driver duration. Completed response usage and actual attempts share the
/// report's own request index; absent partial usage remains absent.
pub(in crate::session) fn models_reported(
    source: &TurnIdentity,
    base: u64,
    report: &LoopReport,
) -> Vec<Record> {
    let responses: BTreeMap<_, _> = report
        .appended
        .iter()
        .filter_map(|item| match item {
            HistoryItem::Assistant(response) => Some((response.request_index as usize, response)),
            _ => None,
        })
        .collect();
    let mut records = Vec::new();
    for (index, &attempts) in report.model_attempts.iter().enumerate() {
        if attempts == 0 {
            continue;
        } // No actual model start occurred.
        let response = responses.get(&index).copied();
        let observed_usage = response.map(|response| response.usage).or_else(|| {
            report
                .partial
                .as_ref()
                .filter(|partial| partial.request_index as usize == index)
                .and_then(|partial| partial.usage)
        });
        let data = || {
            let mut data = EventData::default()
                .context(turn_context(source))
                .attribute("loop_id", report.loop_id.to_string())
                .attribute("request_index", index as u64)
                .attribute("attempt", u64::from(attempts))
                .attribute(
                    "outcome",
                    if response.is_some() {
                        "complete_response"
                    } else {
                        "no_complete_response"
                    },
                );
            if let Some(index) = base.checked_add(index as u64) {
                data = data.attribute("response_index", index);
            }
            if let Some(response) = response {
                data = data.attribute(
                    "finish_reason",
                    match response.finish_reason {
                        ModelFinishReason::Stop => "stop",
                        ModelFinishReason::ToolCalls => "tool_calls",
                        ModelFinishReason::Length => "length",
                        ModelFinishReason::ContentFiltered => "content_filtered",
                        ModelFinishReason::Refused => "refused",
                        ModelFinishReason::Unknown => "unknown",
                    },
                );
            }
            usage(data, observed_usage)
        };
        records.push(Record::new(
            event(
                Level::DEBUG,
                Scope::Runtime,
                "session.runtime",
                "model.finished",
                "Runtime returned the original model request's attempt and response facts",
                MODEL_ATTRIBUTES,
            ),
            data(),
        ));
        if response.is_some() && attempts > 1 {
            // Source-measured retry waits share the report's original request
            // index. Planned deadlines and Host observation times are not used.
            let mut recovered = data();
            if let Some(elapsed) = report.model_retry_wait_elapsed.get(index)
                && let Ok(milliseconds) = u64::try_from(elapsed.as_millis())
            {
                recovered = recovered.attribute("wait_duration_ms", milliseconds);
            }
            records.push(Record::new(
                event(
                    Level::INFO,
                    Scope::Runtime,
                    "session.runtime",
                    "model.recovered",
                    "Runtime produced a complete response after actual retries",
                    MODEL_ATTRIBUTES,
                ),
                recovered,
            ));
        }
    }
    records
}
