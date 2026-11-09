use minicore_host_protocol::{
    AgentId, Fault, HostId, HostIncarnation, LoadId, MessageId, OperationId, SessionId, ToolCallId,
    TurnId,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future};
use tracing::{Instrument, Level, Span, instrument::Instrumented};

pub(crate) const TARGET: &str = "minicore_diagnostics";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Scope {
    Host,
    Agent,
    Session,
    Turn,
    Runtime,
    Client,
}

/// Owned, actual source identities. No IDs, counters, or causal relationship are
/// invented. A context is a complete snapshot, not a patch: a child execution
/// replaces its parent's chain and records any origin relationship in attributes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Context {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<HostId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_incarnation: Option<HostIncarnation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<ToolCallId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_id: Option<LoadId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<OperationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<MessageId>,
}

impl Context {
    /// Build a short-lived tracing span with an owned identity snapshot.
    /// Never retain `Span::enter()` guards across `.await`; use [`Self::instrument`]
    /// or `tracing::Instrument`. These context spans remain available even when
    /// their TRACE-level metadata would normally be below the event filter.
    pub fn span(&self) -> Span {
        if !tracing::span_enabled!(target: TARGET, Level::TRACE) {
            return Span::none();
        }
        let context = serde_json::to_string(self).expect("identity strings are JSON serializable");
        tracing::trace_span!(target: TARGET, "diagnostic.context", context = context.as_str())
    }

    /// Instrument each future poll with this source's owned identity. Spawned
    /// tasks must explicitly instrument their own future, or use in_current_span.
    pub fn instrument<F: Future>(&self, future: F) -> Instrumented<F> {
        future.instrument(self.span())
    }
}

/// The only supported attribute leaves. serde_json::Number cannot represent
/// NaN/infinity; callers must omit unknown or non-finite measurements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogScalar {
    Null,
    String(String),
    Number(serde_json::Number),
    Bool(bool),
}

/// Structured objects and recursive arrays are intentionally not representable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogValue {
    Scalar(LogScalar),
    Array(Vec<LogScalar>),
}

impl From<LogScalar> for LogValue {
    fn from(value: LogScalar) -> Self {
        Self::Scalar(value)
    }
}
impl From<Vec<LogScalar>> for LogValue {
    fn from(value: Vec<LogScalar>) -> Self {
        Self::Array(value)
    }
}
impl From<&str> for LogValue {
    fn from(value: &str) -> Self {
        Self::Scalar(LogScalar::String(value.into()))
    }
}
impl From<String> for LogValue {
    fn from(value: String) -> Self {
        Self::Scalar(LogScalar::String(value))
    }
}
impl From<bool> for LogValue {
    fn from(value: bool) -> Self {
        Self::Scalar(LogScalar::Bool(value))
    }
}
impl From<u64> for LogValue {
    fn from(value: u64) -> Self {
        Self::Scalar(LogScalar::Number(value.into()))
    }
}
impl From<i64> for LogValue {
    fn from(value: i64) -> Self {
        Self::Scalar(LogScalar::Number(value.into()))
    }
}

/// Reviewed source metadata and the names of its permitted scalar attributes.
/// Keep messages as static summaries; never add prompts, scripts, configuration,
/// URLs, headers, raw errors, paths, or other payloads to the attribute contract.
#[derive(Clone, Copy, Debug)]
pub struct EventSpec {
    pub level: Level,
    pub scope: Scope,
    pub component: &'static str,
    pub event: &'static str,
    pub message: &'static str,
    pub attributes: &'static [&'static str],
}

/// Data extracted by the source after filtering. It holds identities and small
/// metadata only, never an owner, request, Runtime, or arbitrary Debug object.
#[derive(Default)]
pub struct EventData {
    context: Option<Context>,
    fault: Option<FaultIdentity>,
    attributes: BTreeMap<String, LogValue>,
}

impl EventData {
    /// Override inherited span context completely, including with an empty one.
    pub fn context(mut self, context: Context) -> Self {
        self.context = Some(context);
        self
    }

    /// Copy only the Fault's original type/code and optional diagnostic ID.
    /// Its message, details, causes and request bodies are never serialized.
    pub fn fault(mut self, fault: &Fault) -> Self {
        self.fault = Some(FaultIdentity {
            fault_type: fault.type_name().into(),
            fault_code: fault.code(),
            diagnostic_id: fault.diagnostic_id().map(str::to_owned),
        });
        self
    }

    /// Unknown keys are discarded against the EventSpec before serialization.
    pub fn attribute(mut self, name: &'static str, value: impl Into<LogValue>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FaultIdentity {
    pub(crate) fault_type: String,
    pub(crate) fault_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) diagnostic_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventPayload {
    pub(crate) scope: Scope,
    pub(crate) component: String,
    pub(crate) event: String,
    pub(crate) message: String,
    pub(crate) context: Option<Context>,
    pub(crate) fault: Option<FaultIdentity>,
    pub(crate) attributes: BTreeMap<String, LogValue>,
}

impl EventSpec {
    /// Filter before running the closure or formatting/serializing its values.
    /// Logging is best-effort and cannot replace returning the primary Fault.
    pub fn emit(&self, data: impl FnOnce() -> EventData) {
        macro_rules! emit_at {
            ($level:expr) => {
                if tracing::enabled!(target: TARGET, $level) {
                    let mut data = data();
                    data.attributes.retain(|key, _| self.attributes.contains(&key.as_str()));
                    let payload = EventPayload {
                        scope: self.scope,
                        component: self.component.into(),
                        event: self.event.into(),
                        message: self.message.into(),
                        context: data.context,
                        fault: data.fault,
                        attributes: data.attributes,
                    };
                    let record = serde_json::to_string(&payload)
                        .expect("typed diagnostic metadata is JSON serializable");
                    tracing::event!(target: TARGET, $level, record = record.as_str());
                }
            };
        }
        match self.level {
            Level::ERROR => emit_at!(Level::ERROR),
            Level::WARN => emit_at!(Level::WARN),
            Level::INFO => emit_at!(Level::INFO),
            Level::DEBUG => emit_at!(Level::DEBUG),
            Level::TRACE => emit_at!(Level::TRACE),
        }
    }
}
