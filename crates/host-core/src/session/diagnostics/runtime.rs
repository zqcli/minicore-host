//! Actual Runtime retry boundaries and returned request facts only.
use super::*;
use minicore_host_runtime::model::{ModelFinishReason, ModelRetryProgress, RetryHint, Usage};

pub(in crate::session) fn model_retry(
    source: &TurnIdentity,
    loop_id: LoopId,
    request_index: u32,
    retry: &ModelRetryProgress,
) -> Record {
    let mut data = EventData::default()
        .context(turn_context(source))
        .fault(&runtime_model_fault(&retry.error))
        .attribute("loop_id", loop_id.to_string())
        .attribute("request_index", u64::from(request_index))
        .attribute("attempt", u64::from(retry.attempt))
        .attribute("max_attempts", u64::from(retry.max_attempts));
    data = data.attribute("response_index", u64::from(request_index));
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

/// The direct model-call return supplies actual attempts and usage. Public
/// output events never substitute for a response or partial.
pub(in crate::session) fn model_reported(
    source: &TurnIdentity,
    loop_id: LoopId,
    request_index: u32,
    report: &minicore_host_runtime::model::ModelCallReport,
) -> Vec<Record> {
    if report.attempts == 0 {
        return Vec::new();
    }
    let response = report.outcome.as_ref().ok();
    let observed_usage = match &report.outcome {
        Ok(response) => response.usage().copied(),
        Err(failure) => failure.usage,
    };
    let data = || {
        let mut data = EventData::default()
            .context(turn_context(source))
            .attribute("loop_id", loop_id.to_string())
            .attribute("request_index", u64::from(request_index))
            .attribute("response_index", u64::from(request_index))
            .attribute("attempt", u64::from(report.attempts))
            .attribute(
                "outcome",
                if response.is_some() {
                    "complete_response"
                } else {
                    "no_complete_response"
                },
            );
        if let Some(response) = response {
            data = data.attribute(
                "finish_reason",
                match response.finish_reason() {
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
    let mut records = vec![Record::new(
        event(
            Level::DEBUG,
            Scope::Runtime,
            "session.runtime",
            "model.finished",
            "Runtime returned the original model request's attempt and response facts",
            MODEL_ATTRIBUTES,
        ),
        data(),
    )];
    if response.is_some() && report.attempts > 1 {
        let mut recovered = data();
        if let Ok(milliseconds) = u64::try_from(report.retry_wait_elapsed.as_millis()) {
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
    records
}
