use crate::error::DiagnosticSummary;
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, time::Duration};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    NotStarted,
    Started,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq, Serialize, Deserialize)]
pub enum ModelValueError {
    #[error("model text is empty or contains disallowed control characters")]
    InvalidText,
    #[error("reasoning content has no artifact")]
    EmptyReasoningContent,
    #[error("model message list must not be empty")]
    EmptyMessages,
    #[error("assistant parts must not be empty")]
    EmptyAssistantParts,
    #[error("tool call arguments must be a JSON object")]
    InvalidToolArguments,
    #[error("tool result does not match a pending assistant tool call")]
    OrphanToolResult,
    #[error("tool result id is duplicated in the current exchange")]
    DuplicateToolResult,
    #[error("assistant tool calls must be completed before the next message")]
    IncompleteToolExchange,
    #[error("model response parts must not be empty")]
    EmptyResponseParts,
    #[error("assistant tool call IDs must be unique")]
    DuplicateToolCallId,
    #[error("assistant tool call indices must be unique")]
    DuplicateToolCallIndex,
    #[error("assistant tool call indices must be contiguous and ordered")]
    InvalidToolCallOrder,
    #[error("model descriptor is invalid")]
    InvalidDescriptor,
    #[error("provider replay format is invalid or payload nesting is unsafe")]
    InvalidProviderReplay,
    #[error("model request tools are invalid or not strictly ordered")]
    InvalidTools,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelErrorKind {
    InvalidRequest,
    Unavailable,
    ProviderUnavailable,
    UpstreamUnclassified,
    AuthMissing,
    AuthRejected,
    RateLimited,
    QuotaExceeded,
    ContextOverflow,
    Timeout,
    TransportUnavailable,
    Cancelled,
    InvalidProviderResponse,
    IncompleteResponse,
    StreamInterrupted,
    RequestOutcomeUnknown,
    UnexpectedToolCall,
    Panicked,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryHint {
    Never,
    Retryable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after: Option<Duration>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetryableWire {
    #[serde(default)]
    retry_after: Option<Duration>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum RetryHintWire {
    Never,
    Retryable(RetryableWire),
}

impl<'de> Deserialize<'de> for RetryHint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = RetryHintWire::deserialize(deserializer)?;
        match wire {
            RetryHintWire::Never => Ok(Self::Never),
            RetryHintWire::Retryable(RetryableWire { retry_after }) => {
                Ok(Self::Retryable { retry_after })
            }
        }
    }
}

/// Structured model error carrying explicit delivery state, retry hint, and diagnostic.
///
/// Deserialization strictly rejects unknown fields and rejects `RetryHint::Retryable`
/// if `delivery` is not `DeliveryState::NotStarted`. In both constructors and
/// deserialization, the inner `diagnostic.retryable` flag is automatically normalized
/// to match `retry_hint` (`true` for `Retryable`, `false` for `Never`).
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct ModelError {
    kind: ModelErrorKind,
    delivery: DeliveryState,
    retry_hint: RetryHint,
    diagnostic: DiagnosticSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upstream: Option<Box<ModelUpstreamDetails>>,
}

/// Provider facts, sanitized by the adapter before crossing the model boundary.
/// The human-readable message remains in `ModelError::diagnostic`.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelUpstreamDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl fmt::Debug for ModelUpstreamDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelUpstreamDetails")
            .field("http_status", &self.http_status)
            .field(
                "provider_type_bytes",
                &self.provider_type.as_ref().map(String::len),
            )
            .field(
                "provider_code_bytes",
                &self.provider_code.as_ref().map(String::len),
            )
            .field(
                "request_id_bytes",
                &self.request_id.as_ref().map(String::len),
            )
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelErrorWire {
    kind: ModelErrorKind,
    delivery: DeliveryState,
    retry_hint: RetryHint,
    diagnostic: DiagnosticSummary,
    #[serde(default)]
    upstream: Option<Box<ModelUpstreamDetails>>,
}

impl<'de> Deserialize<'de> for ModelError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ModelErrorWire::deserialize(deserializer)?;
        let mut error = match wire.retry_hint {
            RetryHint::Retryable { .. } if wire.delivery != DeliveryState::NotStarted => {
                return Err(serde::de::Error::custom(
                    "retryable model error requires delivery state NotStarted",
                ));
            }
            RetryHint::Retryable { retry_after } => {
                Self::not_started(wire.kind, retry_after, wire.diagnostic)
            }
            RetryHint::Never => Self::permanent(wire.kind, wire.delivery, wire.diagnostic),
        };
        error.upstream = wire.upstream;
        Ok(error)
    }
}

impl ModelError {
    /// Constructs a retryable model error with `NotStarted` delivery state.
    ///
    /// The inner diagnostic summary's `retryable` flag is normalized to `true`
    /// to remain strictly consistent with `RetryHint::Retryable`.
    pub fn not_started(
        kind: ModelErrorKind,
        retry_after: Option<Duration>,
        mut diagnostic: DiagnosticSummary,
    ) -> Self {
        diagnostic.retryable = true;
        Self {
            kind,
            delivery: DeliveryState::NotStarted,
            retry_hint: RetryHint::Retryable { retry_after },
            diagnostic,
            upstream: None,
        }
    }

    /// Constructs a non-retryable model error with `Started` delivery state.
    ///
    /// The inner diagnostic summary's `retryable` flag is normalized to `false`
    /// to remain strictly consistent with `RetryHint::Never`.
    pub fn started(kind: ModelErrorKind, mut diagnostic: DiagnosticSummary) -> Self {
        diagnostic.retryable = false;
        Self {
            kind,
            delivery: DeliveryState::Started,
            retry_hint: RetryHint::Never,
            diagnostic,
            upstream: None,
        }
    }

    /// Constructs a non-retryable model error with `Unknown` delivery state.
    ///
    /// The inner diagnostic summary's `retryable` flag is normalized to `false`
    /// to remain strictly consistent with `RetryHint::Never`.
    pub fn unknown(kind: ModelErrorKind, mut diagnostic: DiagnosticSummary) -> Self {
        diagnostic.retryable = false;
        Self {
            kind,
            delivery: DeliveryState::Unknown,
            retry_hint: RetryHint::Never,
            diagnostic,
            upstream: None,
        }
    }

    /// Constructs a permanent non-retryable model error with explicit delivery.
    ///
    /// The inner diagnostic summary's `retryable` flag is normalized to `false`
    /// to remain strictly consistent with `RetryHint::Never`.
    pub fn permanent(
        kind: ModelErrorKind,
        delivery: DeliveryState,
        mut diagnostic: DiagnosticSummary,
    ) -> Self {
        diagnostic.retryable = false;
        Self {
            kind,
            delivery,
            retry_hint: RetryHint::Never,
            diagnostic,
            upstream: None,
        }
    }

    pub const fn kind(&self) -> ModelErrorKind {
        self.kind
    }

    pub const fn delivery(&self) -> DeliveryState {
        self.delivery
    }

    pub const fn retry_hint(&self) -> &RetryHint {
        &self.retry_hint
    }

    pub const fn diagnostic(&self) -> &DiagnosticSummary {
        &self.diagnostic
    }

    pub fn upstream(&self) -> Option<&ModelUpstreamDetails> {
        self.upstream.as_deref()
    }

    pub fn with_upstream(mut self, upstream: ModelUpstreamDetails) -> Self {
        self.upstream = Some(Box::new(upstream));
        self
    }

    /// Observed output changes delivery safety, never the original cause.
    pub(crate) fn after_observation(mut self) -> Self {
        self.delivery = DeliveryState::Started;
        self.retry_hint = RetryHint::Never;
        self.diagnostic.retryable = false;
        self
    }
}

impl fmt::Debug for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelError")
            .field("kind", &self.kind)
            .field("delivery", &self.delivery)
            .field("retry_hint", &self.retry_hint)
            .field("diagnostic", &self.diagnostic)
            .field("upstream", &self.upstream)
            .finish()
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "model error: kind={:?}, delivery={:?}, retry_hint={:?}",
            self.kind, self.delivery, self.retry_hint
        )
    }
}

impl std::error::Error for ModelError {}
