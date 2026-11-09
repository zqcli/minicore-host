//! Error classification follows MiniCore Agent's conservative delivery model.
//! See PROVENANCE.md. Provider free-text and raw reqwest errors are never logged
//! or copied into diagnostics (either can contain credentials or prompt text).

use std::time::{Duration, Instant, SystemTime};

use futures_util::StreamExt;
use minicore_runtime::{
    error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary},
    model::{DeliveryState, ModelCallContext, ModelError, ModelErrorKind},
    value::BoundedText,
};
use serde_json::Value;
use tokio::time::Instant as TokioInstant;

pub(crate) fn local(kind: ModelErrorKind) -> ModelError {
    ModelError::permanent(kind, DeliveryState::NotStarted, diagnostic(kind, None))
}
pub(crate) fn unknown(kind: ModelErrorKind) -> ModelError {
    ModelError::unknown(kind, diagnostic(kind, None))
}
pub(crate) fn stream(kind: ModelErrorKind, seen: bool) -> ModelError {
    if seen {
        ModelError::started(kind, diagnostic(kind, None))
    } else {
        unknown(kind)
    }
}
fn retryable(kind: ModelErrorKind, delay: Option<Duration>, status: Option<u16>) -> ModelError {
    ModelError::not_started(kind, delay, diagnostic(kind, status))
}

pub(crate) fn send(error: reqwest::Error) -> ModelError {
    if error.is_builder() {
        local(ModelErrorKind::InvalidRequest)
    } else if error.is_connect() {
        retryable(ModelErrorKind::TransportUnavailable, None, None)
    } else if error.is_timeout() {
        unknown(ModelErrorKind::Timeout)
    } else {
        unknown(ModelErrorKind::RequestOutcomeUnknown)
    }
}

fn diagnostic(kind: ModelErrorKind, status: Option<u16>) -> DiagnosticSummary {
    let code = match kind {
        ModelErrorKind::InvalidRequest | ModelErrorKind::ContextOverflow => {
            DiagnosticCode::InvalidConfiguration
        }
        ModelErrorKind::Cancelled => DiagnosticCode::RuntimeTerminated,
        ModelErrorKind::Timeout => DiagnosticCode::ModelTimeout,
        ModelErrorKind::InvalidProviderResponse
        | ModelErrorKind::IncompleteResponse
        | ModelErrorKind::StreamInterrupted
        | ModelErrorKind::UnexpectedToolCall => DiagnosticCode::ModelMalformedResponse,
        ModelErrorKind::Internal | ModelErrorKind::Panicked => DiagnosticCode::Internal,
        _ => DiagnosticCode::ModelUnavailable,
    };
    let text = match status {
        Some(status) => format!("provider HTTP {status}: {kind:?}"),
        None => format!("provider request: {kind:?}"),
    };
    DiagnosticSummary::new(
        code,
        DiagnosticCategory::Model,
        BoundedText::new(text).expect("static diagnostic is bounded"),
        false,
    )
    .expect("static diagnostic is valid")
}

fn structured_kind(value: &Value) -> Option<ModelErrorKind> {
    let error = value.get("error").unwrap_or(value);
    let codes: Vec<_> = ["code", "type"]
        .into_iter()
        .filter_map(|key| error.get(key).and_then(Value::as_str))
        .collect();
    // A hard quota always wins over a generic rate-limit type.
    if codes.iter().any(|code| {
        matches!(
            *code,
            "insufficient_quota"
                | "quota_exceeded"
                | "credit_balance_exhausted"
                | "billing_hard_limit_reached"
                | "billing_hard_limit_exceeded"
                | "billing_limit_reached"
                | "usage_limit_reached"
                | "usage_limit_exceeded"
                | "organization_quota_exceeded"
                | "project_quota_exceeded"
                | "budget_exceeded"
                | "spend_limit_exceeded"
                | "spend_limit_reached"
                | "organization_budget_exceeded"
                | "project_budget_exceeded"
                | "organization_usage_limit_reached"
                | "organization_usage_limit_exceeded"
                | "project_usage_limit_reached"
                | "project_usage_limit_exceeded"
                | "organization_spend_limit_reached"
                | "organization_spend_limit_exceeded"
                | "project_spend_limit_reached"
                | "project_spend_limit_exceeded"
                | "organization_limit_reached"
                | "organization_limit_exceeded"
                | "project_limit_reached"
                | "project_limit_exceeded"
        )
    }) {
        return Some(ModelErrorKind::QuotaExceeded);
    }
    codes.into_iter().find_map(|code| match code {
        "context_length_exceeded" | "context_window_exceeded" | "context_overflow" => {
            Some(ModelErrorKind::ContextOverflow)
        }
        "rate_limit_exceeded" | "rate_limit_error" => Some(ModelErrorKind::RateLimited),
        "overloaded_error" | "server_overloaded" | "model_overloaded" => {
            Some(ModelErrorKind::ProviderUnavailable)
        }
        "invalid_api_key" | "authentication_error" => Some(ModelErrorKind::AuthRejected),
        "invalid_request_error" => Some(ModelErrorKind::InvalidRequest),
        _ => None,
    })
}

pub(crate) fn provider(
    value: &Value,
    seen: bool,
    status: u16,
    retry_at: Option<Instant>,
) -> ModelError {
    let recognized = structured_kind(value);
    let kind = recognized.unwrap_or(ModelErrorKind::ProviderUnavailable);
    let diagnostic = diagnostic(kind, Some(status));
    if seen {
        return ModelError::started(kind, diagnostic);
    }
    match recognized {
        Some(ModelErrorKind::RateLimited | ModelErrorKind::ProviderUnavailable) => {
            ModelError::not_started(
                kind,
                retry_at.and_then(|at| at.checked_duration_since(Instant::now())),
                diagnostic,
            )
        }
        Some(_) => ModelError::permanent(kind, DeliveryState::NotStarted, diagnostic),
        None => ModelError::unknown(kind, diagnostic),
    }
}

pub(crate) async fn http(
    response: reqwest::Response,
    context: &ModelCallContext,
    deadline: TokioInstant,
) -> ModelError {
    let status = response.status().as_u16();
    let retry_at = retry_after(response.headers());
    let mut bytes = response.bytes_stream();
    let mut body = Vec::new();
    loop {
        let next = tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => return unknown(ModelErrorKind::Cancelled),
            _ = tokio::time::sleep_until(deadline) => return unknown(ModelErrorKind::Timeout),
            next = bytes.next() => next,
        };
        match next {
            Some(Ok(chunk)) => {
                // Bound diagnostic extraction only. An oversized error body is
                // discarded and remains unclassified, never a retry license.
                if body
                    .len()
                    .checked_add(chunk.len())
                    .is_none_or(|size| size > BoundedText::MAX_BYTES)
                {
                    body.clear();
                    break;
                }
                body.extend_from_slice(&chunk);
            }
            Some(Err(error)) if error.is_timeout() => return unknown(ModelErrorKind::Timeout),
            Some(Err(_)) => return unknown(ModelErrorKind::RequestOutcomeUnknown),
            None => break,
        }
    }
    let recognized = serde_json::from_slice::<Value>(&body)
        .ok()
        .as_ref()
        .and_then(structured_kind);
    let (kind, delivery, retry) = match (status, recognized) {
        (_, Some(ModelErrorKind::QuotaExceeded)) => (
            ModelErrorKind::QuotaExceeded,
            DeliveryState::NotStarted,
            false,
        ),
        (400 | 422, Some(ModelErrorKind::ContextOverflow)) => (
            ModelErrorKind::ContextOverflow,
            DeliveryState::NotStarted,
            false,
        ),
        (401 | 403, _) => (
            ModelErrorKind::AuthRejected,
            DeliveryState::NotStarted,
            false,
        ),
        (429, Some(ModelErrorKind::RateLimited)) => {
            (ModelErrorKind::RateLimited, DeliveryState::NotStarted, true)
        }
        (503 | 529, Some(ModelErrorKind::ProviderUnavailable)) => (
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::NotStarted,
            true,
        ),
        (429, _) => (
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::Unknown,
            false,
        ),
        (408, _) => (ModelErrorKind::Timeout, DeliveryState::Unknown, false),
        (400..=499, _) => (
            recognized.unwrap_or(ModelErrorKind::InvalidRequest),
            DeliveryState::NotStarted,
            false,
        ),
        _ => (
            recognized.unwrap_or(ModelErrorKind::ProviderUnavailable),
            DeliveryState::Unknown,
            false,
        ),
    };
    if retry {
        retryable(
            kind,
            retry_at.and_then(|at| at.checked_duration_since(Instant::now())),
            Some(status),
        )
    } else {
        ModelError::permanent(kind, delivery, diagnostic(kind, Some(status)))
    }
}

pub(crate) fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Instant> {
    let now = Instant::now();
    let decimal = |value: &str, divisor: f64| {
        Duration::try_from_secs_f64(value.trim().parse::<f64>().ok()? / divisor).ok()
    };
    let duration = headers
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| decimal(value, 1000.0))
        .or_else(|| {
            let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
            decimal(value, 1.0).or_else(|| {
                httpdate::parse_http_date(value)
                    .ok()?
                    .duration_since(SystemTime::now())
                    .ok()
            })
        })?;
    now.checked_add(duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderMap;

    #[test]
    fn retry_after_honors_milliseconds_seconds_and_http_dates() {
        for (name, value) in [("retry-after-ms", "2500"), ("retry-after", "2.5")] {
            let mut headers = HeaderMap::new();
            headers.insert(name, value.parse().unwrap());
            let delay = retry_after(&headers)
                .unwrap()
                .checked_duration_since(Instant::now())
                .unwrap();
            assert!(delay > Duration::from_secs(2) && delay <= Duration::from_millis(2500));
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            "retry-after",
            httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(60))
                .parse()
                .unwrap(),
        );
        let delay = retry_after(&headers)
            .unwrap()
            .checked_duration_since(Instant::now())
            .unwrap();
        assert!(delay > Duration::from_secs(58) && delay <= Duration::from_secs(60));
        headers.insert("retry-after", "-1".parse().unwrap());
        assert!(retry_after(&headers).is_none());
        headers.insert("retry-after", "NaN".parse().unwrap());
        assert!(retry_after(&headers).is_none());
    }
}
