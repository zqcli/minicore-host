use crate::{ValidationError, invalid};
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FaultClass {
    Host,
    HostAccess,
    HostStorage,
    HostLookup,
    Agent,
    AgentConfig,
    AgentLifecycle,
    AgentLookup,
    Session,
    SessionLifecycle,
    SessionHistory,
    SessionInput,
    SessionConfig,
    SessionLookup,
    Turn,
    TurnModel,
    TurnModelUpstream,
    TurnModelStream,
}

/// One registry owns the public type names, stable numeric codes, and explicit
/// classification ancestry. Underscores do not invent hierarchy nodes.
macro_rules! fault_registry {
    ($($variant:ident = $code:literal, $name:literal, [$($parent:ident),+];)+) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
        pub enum FaultKind { $(#[serde(rename = $name)] $variant,)+ }
        impl FaultKind {
            pub const fn code(self) -> u16 { match self { $(Self::$variant => $code,)+ } }
            pub const fn type_name(self) -> &'static str { match self { $(Self::$variant => $name,)+ } }
            pub const fn classes(self) -> &'static [FaultClass] {
                match self { $(Self::$variant => &[$(FaultClass::$parent),+],)+ }
            }
            pub fn is_a(self, class: FaultClass) -> bool { self.classes().contains(&class) }
        }
        pub const FAULT_KINDS: &[FaultKind] = &[$(FaultKind::$variant,)+];
    };
}
fault_registry! {
    HostAccessInvalid = 1001, "HOST_ACCESS_INVALID", [Host, HostAccess];
    HostAccessReadOnly = 1002, "HOST_ACCESS_READ_ONLY", [Host, HostAccess];
    HostOwnershipMismatch = 1003, "HOST_OWNERSHIP_MISMATCH", [Host];
    HostStorageUnavailable = 1101, "HOST_STORAGE_UNAVAILABLE", [Host, HostStorage];
    HostAgentNotFound = 1201, "HOST_AGENT_NOT_FOUND", [Host, HostLookup];
    HostLifecycleUnavailable = 1301, "HOST_LIFECYCLE_UNAVAILABLE", [Host];
    HostMethodUnavailable = 1401, "HOST_METHOD_UNAVAILABLE", [Host];
    HostInternalUnexpected = 1999, "HOST_INTERNAL_UNEXPECTED", [Host];
    AgentConfigInvalid = 2101, "AGENT_CONFIG_INVALID", [Agent, AgentConfig];
    AgentConfigConflict = 2102, "AGENT_CONFIG_CONFLICT", [Agent, AgentConfig];
    AgentLifecycleUnavailable = 2201, "AGENT_LIFECYCLE_UNAVAILABLE", [Agent, AgentLifecycle];
    AgentSessionNotFound = 2202, "AGENT_SESSION_NOT_FOUND", [Agent, AgentLookup];
    AgentDefaultSessionUnavailable = 2203, "AGENT_DEFAULT_SESSION_UNAVAILABLE", [Agent, AgentLookup];
    AgentOwnershipMismatch = 2204, "AGENT_OWNERSHIP_MISMATCH", [Agent];
    AgentInternalUnexpected = 2999, "AGENT_INTERNAL_UNEXPECTED", [Agent];
    SessionLifecycleClosed = 3101, "SESSION_LIFECYCLE_CLOSED", [Session, SessionLifecycle];
    SessionLifecycleClosing = 3102, "SESSION_LIFECYCLE_CLOSING", [Session, SessionLifecycle];
    SessionBusy = 3103, "SESSION_BUSY", [Session];
    SessionTurnNotFound = 3201, "SESSION_TURN_NOT_FOUND", [Session, SessionLookup];
    SessionInputNotFound = 3202, "SESSION_INPUT_NOT_FOUND", [Session, SessionInput];
    SessionInputAlreadyBound = 3203, "SESSION_INPUT_ALREADY_BOUND", [Session, SessionInput];
    SessionInputInvalid = 3204, "SESSION_INPUT_INVALID", [Session, SessionInput];
    SessionOwnershipMismatch = 3205, "SESSION_OWNERSHIP_MISMATCH", [Session];
    SessionHistoryWriteFailed = 3301, "SESSION_HISTORY_WRITE_FAILED", [Session, SessionHistory];
    SessionHistoryLoadFailed = 3302, "SESSION_HISTORY_LOAD_FAILED", [Session, SessionHistory];
    SessionConfigInvalid = 3401, "SESSION_CONFIG_INVALID", [Session, SessionConfig];
    SessionConfigConflict = 3402, "SESSION_CONFIG_CONFLICT", [Session, SessionConfig];
    SessionResourceMissing = 3403, "SESSION_RESOURCE_MISSING", [Session, SessionConfig];
    SessionInternalUnexpected = 3999, "SESSION_INTERNAL_UNEXPECTED", [Session];
    TurnModelUpstreamRateLimited = 4201, "TURN_MODEL_UPSTREAM_RATE_LIMITED", [Turn, TurnModel, TurnModelUpstream];
    TurnModelUpstreamOverloaded = 4202, "TURN_MODEL_UPSTREAM_OVERLOADED", [Turn, TurnModel, TurnModelUpstream];
    TurnModelUpstreamQuotaExhausted = 4203, "TURN_MODEL_UPSTREAM_QUOTA_EXHAUSTED", [Turn, TurnModel, TurnModelUpstream];
    TurnModelUpstreamAuthenticationFailed = 4204, "TURN_MODEL_UPSTREAM_AUTHENTICATION_FAILED", [Turn, TurnModel, TurnModelUpstream];
    TurnModelStreamInterrupted = 4210, "TURN_MODEL_STREAM_INTERRUPTED", [Turn, TurnModel, TurnModelStream];
    TurnModelUpstreamUnclassified = 4299, "TURN_MODEL_UPSTREAM_UNCLASSIFIED", [Turn, TurnModel, TurnModelUpstream];
    TurnInternalUnexpected = 4999, "TURN_INTERNAL_UNEXPECTED", [Turn];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryWriteStage {
    Write,
    Sync,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryWriteDetails {
    pub stage: HistoryWriteStage,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryLoadDetails {
    /// One-based line when actually known, never guessed from an observer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<u64>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamDetails {
    /// Sanitized provider identifiers, never raw responses or credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_code: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum FaultDetails {
    HistoryWrite(HistoryWriteDetails),
    HistoryLoad(HistoryLoadDetails),
    Upstream(UpstreamDetails),
}

/// Closed public business-fault contract. The numeric code is derived from the
/// registry on serialization and checked on input; callers cannot mismatch it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fault {
    kind: FaultKind,
    message: String,
    diagnostic_id: Option<String>,
    details: Option<FaultDetails>,
    retry_after_ms: Option<u64>,
}
impl Fault {
    /// The owner supplies a safe human-readable summary, not a raw error body.
    pub fn new(kind: FaultKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            diagnostic_id: None,
            details: None,
            retry_after_ms: None,
        }
    }
    pub const fn kind(&self) -> FaultKind {
        self.kind
    }
    pub const fn code(&self) -> u16 {
        self.kind.code()
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn diagnostic_id(&self) -> Option<&str> {
        self.diagnostic_id.as_deref()
    }
    pub fn details(&self) -> Option<&FaultDetails> {
        self.details.as_ref()
    }
    pub const fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
    }
    pub fn with_diagnostic_id(mut self, diagnostic_id: impl Into<String>) -> Self {
        self.diagnostic_id = Some(diagnostic_id.into());
        self
    }
    pub fn with_details(mut self, details: FaultDetails) -> Result<Self, ValidationError> {
        self.details = Some(details);
        self.validate()?;
        Ok(self)
    }
    /// A waiting suggestion, never authorization to repeat any operation.
    pub fn with_retry_after_ms(mut self, value: u64) -> Result<Self, ValidationError> {
        self.retry_after_ms = Some(value);
        self.validate()?;
        Ok(self)
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.retry_after_ms.is_some()
            && !matches!(
                self.kind,
                FaultKind::TurnModelUpstreamRateLimited | FaultKind::TurnModelUpstreamOverloaded
            )
        {
            return Err(invalid(
                "retry_after_ms is only registered for temporary upstream faults",
            ));
        }
        match &self.details {
            None => {}
            Some(FaultDetails::HistoryWrite(_))
                if self.kind == FaultKind::SessionHistoryWriteFailed => {}
            Some(FaultDetails::HistoryLoad(location))
                if self.kind == FaultKind::SessionHistoryLoadFailed =>
            {
                if location.line == Some(0) {
                    return Err(invalid("history location line is one-based"));
                }
            }
            Some(FaultDetails::Upstream(_)) if self.kind.is_a(FaultClass::TurnModelUpstream) => {}
            Some(_) => return Err(invalid("details do not match the registered fault type")),
        }
        Ok(())
    }
}
impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}): {}",
            self.kind.type_name(),
            self.code(),
            self.message
        )
    }
}
impl std::error::Error for Fault {}
impl Serialize for Fault {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut state = s.serialize_struct(
            "Fault",
            3 + usize::from(self.diagnostic_id.is_some())
                + usize::from(self.details.is_some())
                + usize::from(self.retry_after_ms.is_some()),
        )?;
        state.serialize_field("type", &self.kind)?;
        state.serialize_field("code", &self.code())?;
        state.serialize_field("message", &self.message)?;
        if let Some(value) = &self.diagnostic_id {
            state.serialize_field("diagnostic_id", value)?;
        }
        if let Some(value) = &self.details {
            state.serialize_field("details", value)?;
        }
        if let Some(value) = self.retry_after_ms {
            state.serialize_field("retry_after_ms", &value)?;
        }
        state.end()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultWire {
    #[serde(rename = "type")]
    kind: FaultKind,
    code: u16,
    message: String,
    diagnostic_id: Option<String>,
    details: Option<serde_json::Value>,
    retry_after_ms: Option<u64>,
}
impl<'de> Deserialize<'de> for Fault {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = FaultWire::deserialize(d)?;
        if wire.code != wire.kind.code() {
            return Err(serde::de::Error::custom(
                "fault type/code registry mismatch",
            ));
        }
        let details = wire
            .details
            .map(|value| {
                match wire.kind {
                    FaultKind::SessionHistoryWriteFailed => {
                        serde_json::from_value(value).map(FaultDetails::HistoryWrite)
                    }
                    FaultKind::SessionHistoryLoadFailed => {
                        serde_json::from_value(value).map(FaultDetails::HistoryLoad)
                    }
                    kind if kind.is_a(FaultClass::TurnModelUpstream) => {
                        serde_json::from_value(value).map(FaultDetails::Upstream)
                    }
                    _ => {
                        return Err(serde::de::Error::custom(
                            "this fault type has no registered details",
                        ));
                    }
                }
                .map_err(serde::de::Error::custom)
            })
            .transpose()?;
        let fault = Self {
            kind: wire.kind,
            message: wire.message,
            diagnostic_id: wire.diagnostic_id,
            details,
            retry_after_ms: wire.retry_after_ms,
        };
        fault.validate().map_err(serde::de::Error::custom)?;
        Ok(fault)
    }
}

/// Client-side errors preserve provenance. Timeout/connection/protocol failure
/// never fabricates a server receipt, Fault, or execution outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallError {
    Business(Fault),
    WaitTimeout { wait_timeout_ms: u64 },
    Connection { message: String },
    Protocol { message: String },
}
impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Business(fault) => fault.fmt(f),
            Self::WaitTimeout { .. } => f.write_str(
                "waiting for a reply timed out; operation confirmation was not obtained",
            ),
            Self::Connection { message } | Self::Protocol { message } => message.fmt(f),
        }
    }
}
impl std::error::Error for CallError {}
impl From<Fault> for CallError {
    fn from(value: Fault) -> Self {
        Self::Business(value)
    }
}
