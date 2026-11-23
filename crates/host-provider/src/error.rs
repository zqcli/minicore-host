//! Error classification follows MiniCore Agent's conservative delivery model.
//! See PROVENANCE.md. Only a credential-redacted provider message and selected
//! source facts cross the model boundary. Raw responses and reqwest errors are
//! never copied into diagnostics; diagnostic events do not record message text.

use std::time::{Duration, Instant, SystemTime};

use futures_util::StreamExt;
use minicore_host_runtime::{
    error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary},
    model::{DeliveryState, ModelCallContext, ModelError, ModelErrorKind, ModelUpstreamDetails},
    value::Text,
};
use serde_json::Value;
use tokio::time::Instant as TokioInstant;

/// Response facts and the credential bound to this exact attempt. This remains
/// provider-private and is never serialized or exposed by Debug.
pub(crate) struct ResponseContext {
    pub(crate) status: u16,
    pub(crate) retry_at: Option<Instant>,
    request_id: Option<String>,
    authorization: reqwest::header::HeaderValue,
}

impl ResponseContext {
    #[cfg(test)]
    pub(crate) fn fixture(status: u16) -> Self {
        let mut authorization: reqwest::header::HeaderValue = "Bearer fixture-key".parse().unwrap();
        authorization.set_sensitive(true);
        Self {
            status,
            retry_at: None,
            request_id: None,
            authorization,
        }
    }

    pub(crate) fn new(
        response: &reqwest::Response,
        authorization: &reqwest::header::HeaderValue,
    ) -> Self {
        let mut context = Self {
            status: response.status().as_u16(),
            retry_at: retry_after(response.headers()),
            request_id: None,
            authorization: authorization.clone(),
        };
        context.request_id = ["x-request-id", "request-id"]
            .into_iter()
            .find_map(|name| {
                response
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
            })
            .and_then(|value| context.text(value));
        context
    }

    pub(crate) fn text(&self, value: &str) -> Option<String> {
        if value.trim().is_empty() {
            return None;
        }
        // Redact before control escaping so a credential with punctuation is
        // matched exactly. The sensitive header is the original Turn binding.
        let credential = self
            .authorization
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "));
        let value = match credential.filter(|value| !value.is_empty()) {
            Some(credential) => value.replace(credential, "[REDACTED]"),
            None => value.to_owned(),
        };
        // DiagnosticSummary accepts LF/TAB but no other control characters.
        // Show those characters explicitly without dropping the useful message.
        let mut safe = String::with_capacity(value.len());
        for character in value.chars() {
            if character.is_control() && !matches!(character, '\n' | '\t') {
                safe.extend(character.escape_default());
            } else {
                safe.push(character);
            }
        }
        Some(safe)
    }

    fn field(&self, value: Option<&Value>, name: &str) -> Option<String> {
        value
            .and_then(|value| value.get(name))
            .and_then(Value::as_str)
            .and_then(|value| self.text(value))
    }

    pub(crate) fn record_request_id(&mut self, value: Option<&str>) {
        if self.request_id.is_none() {
            self.request_id = value.and_then(|value| self.text(value));
        }
    }

    fn diagnostic(&self, kind: ModelErrorKind, value: Option<&Value>) -> DiagnosticSummary {
        self.diagnostic_with_message(kind, self.field(value, "message"), value)
    }

    fn diagnostic_with_message(
        &self,
        kind: ModelErrorKind,
        message: Option<String>,
        value: Option<&Value>,
    ) -> DiagnosticSummary {
        let mut message = match message {
            Some(message) => format!("provider HTTP {}: {message}", self.status),
            None => format!("provider HTTP {}: {kind:?}", self.status),
        };
        let details = self.details(value);
        for (name, value) in [
            ("provider_type", details.provider_type),
            ("provider_code", details.provider_code),
            ("request_id", details.request_id),
        ] {
            if let Some(value) = value {
                message.push('\n');
                message.push_str(name);
                message.push_str(": ");
                message.push_str(&value);
            }
        }
        diagnostic_message(kind, Some(self.status), Some(message))
    }

    fn details(&self, value: Option<&Value>) -> ModelUpstreamDetails {
        ModelUpstreamDetails {
            http_status: Some(self.status),
            provider_type: self.field(value, "type"),
            provider_code: self.field(value, "code"),
            request_id: self
                .request_id
                .clone()
                .or_else(|| self.field(value, "request_id")),
        }
    }

    pub(crate) fn attach(&self, error: ModelError) -> ModelError {
        if error.upstream().is_some() {
            error
        } else {
            let diagnostic = self.diagnostic(error.kind(), None);
            let error = match *error.retry_hint() {
                minicore_host_runtime::model::RetryHint::Never => {
                    ModelError::permanent(error.kind(), error.delivery(), diagnostic)
                }
                minicore_host_runtime::model::RetryHint::Retryable { retry_after } => {
                    ModelError::not_started(error.kind(), retry_after, diagnostic)
                }
            };
            error.with_upstream(self.details(None))
        }
    }

    pub(crate) fn incomplete(&self, reason: Option<&str>, seen: bool) -> ModelError {
        let message = reason
            .and_then(|value| self.text(value))
            .map(|reason| format!("response incomplete: {reason}"));
        let diagnostic =
            self.diagnostic_with_message(ModelErrorKind::IncompleteResponse, message, None);
        let error = if seen {
            ModelError::started(ModelErrorKind::IncompleteResponse, diagnostic)
        } else {
            ModelError::unknown(ModelErrorKind::IncompleteResponse, diagnostic)
        };
        error.with_upstream(self.details(None))
    }
}

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
    diagnostic_message(kind, status, None)
}

fn diagnostic_message(
    kind: ModelErrorKind,
    status: Option<u16>,
    message: Option<String>,
) -> DiagnosticSummary {
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
    let text = message.unwrap_or_else(|| match status {
        Some(status) => format!("provider HTTP {status}: {kind:?}"),
        None => format!("provider request: {kind:?}"),
    });
    DiagnosticSummary::new(code, DiagnosticCategory::Model, Text::new(text), false)
        .expect("static diagnostic is valid")
}

fn structured_kind(error: &Value) -> Option<ModelErrorKind> {
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
        // Match the specific overload identifier, not the broader
        // service_unavailable_error type: that alone does not prove safe delivery.
        "overloaded_error" | "server_overloaded" | "server_is_overloaded" | "model_overloaded" => {
            Some(ModelErrorKind::ProviderUnavailable)
        }
        "invalid_api_key" | "authentication_error" => Some(ModelErrorKind::AuthRejected),
        "invalid_request_error" => Some(ModelErrorKind::InvalidRequest),
        _ => None,
    })
}

pub(crate) fn provider(value: &Value, seen: bool, context: &ResponseContext) -> ModelError {
    let recognized = structured_kind(value);
    let kind = recognized.unwrap_or(ModelErrorKind::UpstreamUnclassified);
    let diagnostic = context.diagnostic(kind, Some(value));
    let error = if seen {
        ModelError::started(kind, diagnostic)
    } else {
        match recognized {
            Some(ModelErrorKind::RateLimited | ModelErrorKind::ProviderUnavailable) => {
                ModelError::not_started(
                    kind,
                    context
                        .retry_at
                        .and_then(|at| at.checked_duration_since(Instant::now())),
                    diagnostic,
                )
            }
            Some(_) => ModelError::permanent(kind, DeliveryState::NotStarted, diagnostic),
            None => ModelError::unknown(kind, diagnostic),
        }
    };
    error.with_upstream(context.details(Some(value)))
}

pub(crate) async fn http(
    response: reqwest::Response,
    context: &ModelCallContext,
    deadline: TokioInstant,
    authorization: &reqwest::header::HeaderValue,
) -> ModelError {
    let mut source = ResponseContext::new(&response, authorization);
    let status = source.status;
    let mut bytes = response.bytes_stream();
    let mut body = Vec::new();
    loop {
        let next = tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => return source.attach(unknown(ModelErrorKind::Cancelled)),
            _ = tokio::time::sleep_until(deadline) => return source.attach(unknown(ModelErrorKind::Timeout)),
            next = bytes.next() => next,
        };
        match next {
            Some(Ok(chunk)) => {
                body.extend_from_slice(&chunk);
            }
            Some(Err(error)) if error.is_timeout() => {
                return source.attach(unknown(ModelErrorKind::Timeout));
            }
            Some(Err(_)) => return source.attach(unknown(ModelErrorKind::RequestOutcomeUnknown)),
            None => break,
        }
    }
    let envelope = serde_json::from_slice::<Value>(&body).ok();
    source.record_request_id(
        envelope
            .as_ref()
            .and_then(|value| value.get("request_id"))
            .and_then(Value::as_str),
    );
    let value = envelope
        .as_ref()
        .map(|value| value.get("error").unwrap_or(value));
    let recognized = value.and_then(structured_kind);
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
        (503 | 529, Some(ModelErrorKind::ProviderUnavailable)) => (
            ModelErrorKind::ProviderUnavailable,
            DeliveryState::NotStarted,
            true,
        ),
        // A 429 rejects the request before processing whatever its body says.
        (429, _) => (ModelErrorKind::RateLimited, DeliveryState::NotStarted, true),
        (408, _) => (ModelErrorKind::Timeout, DeliveryState::Unknown, false),
        (400..=499, _) => (
            recognized.unwrap_or(ModelErrorKind::InvalidRequest),
            DeliveryState::NotStarted,
            false,
        ),
        _ => (
            recognized.unwrap_or(ModelErrorKind::UpstreamUnclassified),
            DeliveryState::Unknown,
            false,
        ),
    };
    let diagnostic = source.diagnostic(kind, value);
    let error = if retry {
        ModelError::not_started(
            kind,
            source
                .retry_at
                .and_then(|at| at.checked_duration_since(Instant::now())),
            diagnostic,
        )
    } else {
        ModelError::permanent(kind, delivery, diagnostic)
    };
    error.with_upstream(source.details(value))
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
    fn official_overload_preserves_started_delivery_and_quota_priority() {
        use minicore_host_runtime::model::RetryHint;
        use serde_json::json;

        let overload = json!({"type":"service_unavailable_error","code":"server_is_overloaded"});
        let context = ResponseContext::fixture(200);
        let started = provider(&overload, true, &context);
        assert_eq!(started.kind(), ModelErrorKind::ProviderUnavailable);
        assert_eq!(started.delivery(), DeliveryState::Started);
        assert_eq!(*started.retry_hint(), RetryHint::Never);

        // Keep the existing code/type compatibility and hard-quota precedence.
        assert_eq!(
            structured_kind(&json!({"type":"server_is_overloaded"})),
            Some(ModelErrorKind::ProviderUnavailable)
        );
        for error in [
            json!({"type":"insufficient_quota","code":"server_is_overloaded"}),
            json!({"type":"server_is_overloaded","code":"insufficient_quota"}),
        ] {
            let error = provider(&error, false, &context);
            assert_eq!(error.kind(), ModelErrorKind::QuotaExceeded);
            assert_eq!(error.delivery(), DeliveryState::NotStarted);
            assert_eq!(*error.retry_hint(), RetryHint::Never);
        }
    }

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
