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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultDetailsSchema {
    None,
    HistoryWrite,
    HistoryLoad,
    Upstream,
}

/// One registry owns names, codes, explicit ancestry, permitted details and wait
/// hints. Underscores do not invent hierarchy nodes. Details are optional, but
/// when present must follow the selected typed schema's required fields.
macro_rules! fault_registry {
    ($($variant:ident = $code:literal, $name:literal, [$($parent:ident),+], $details:ident, $retry:literal;)+) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
        pub enum FaultKind { $(#[serde(rename = $name)] $variant,)+ }
        impl FaultKind {
            pub const fn code(self) -> u16 { match self { $(Self::$variant => $code,)+ } }
            pub const fn type_name(self) -> &'static str { match self { $(Self::$variant => $name,)+ } }
            pub const fn classes(self) -> &'static [FaultClass] {
                match self { $(Self::$variant => &[$(FaultClass::$parent),+],)+ }
            }
            pub fn is_a(self, class: FaultClass) -> bool { self.classes().contains(&class) }
            pub const fn details_schema(self) -> FaultDetailsSchema { match self { $(Self::$variant => FaultDetailsSchema::$details,)+ } }
            pub const fn allows_retry_after_ms(self) -> bool { match self { $(Self::$variant => $retry,)+ } }
        }
        pub const FAULT_KINDS: &[FaultKind] = &[$(FaultKind::$variant,)+];
    };
}
fault_registry! {
    HostAccessInvalid = 1001, "HOST_ACCESS_INVALID", [Host, HostAccess], None, false;
    HostAccessReadOnly = 1002, "HOST_ACCESS_READ_ONLY", [Host, HostAccess], None, false;
    HostOwnershipMismatch = 1003, "HOST_OWNERSHIP_MISMATCH", [Host], None, false;
    HostStorageUnavailable = 1101, "HOST_STORAGE_UNAVAILABLE", [Host, HostStorage], None, false;
    HostAgentNotFound = 1201, "HOST_AGENT_NOT_FOUND", [Host, HostLookup], None, false;
    HostReceiptNotFound = 1202, "HOST_RECEIPT_NOT_FOUND", [Host, HostLookup], None, false;
    HostReceiptAmbiguous = 1203, "HOST_RECEIPT_AMBIGUOUS", [Host, HostLookup], None, false;
    HostLifecycleUnavailable = 1301, "HOST_LIFECYCLE_UNAVAILABLE", [Host], None, false;
    HostMethodUnavailable = 1401, "HOST_METHOD_UNAVAILABLE", [Host], None, false;
    HostObservationInvalid = 1501, "HOST_OBSERVATION_INVALID", [Host], None, false;
    HostSubscriptionNotFound = 1502, "HOST_SUBSCRIPTION_NOT_FOUND", [Host, HostLookup], None, false;
    HostObservationResyncRequired = 1503, "HOST_OBSERVATION_RESYNC_REQUIRED", [Host], None, false;
    HostInternalUnexpected = 1999, "HOST_INTERNAL_UNEXPECTED", [Host], None, false;
    AgentConfigInvalid = 2101, "AGENT_CONFIG_INVALID", [Agent, AgentConfig], None, false;
    AgentConfigConflict = 2102, "AGENT_CONFIG_CONFLICT", [Agent, AgentConfig], None, false;
    AgentLifecycleUnavailable = 2201, "AGENT_LIFECYCLE_UNAVAILABLE", [Agent, AgentLifecycle], None, false;
    AgentSessionNotFound = 2202, "AGENT_SESSION_NOT_FOUND", [Agent, AgentLookup], None, false;
    AgentDefaultSessionUnavailable = 2203, "AGENT_DEFAULT_SESSION_UNAVAILABLE", [Agent, AgentLookup], None, false;
    AgentOwnershipMismatch = 2204, "AGENT_OWNERSHIP_MISMATCH", [Agent], None, false;
    AgentInternalUnexpected = 2999, "AGENT_INTERNAL_UNEXPECTED", [Agent], None, false;
    SessionLifecycleClosed = 3101, "SESSION_LIFECYCLE_CLOSED", [Session, SessionLifecycle], None, false;
    SessionLifecycleClosing = 3102, "SESSION_LIFECYCLE_CLOSING", [Session, SessionLifecycle], None, false;
    SessionBusy = 3103, "SESSION_BUSY", [Session], None, false;
    SessionTurnNotFound = 3201, "SESSION_TURN_NOT_FOUND", [Session, SessionLookup], None, false;
    SessionInputNotFound = 3202, "SESSION_INPUT_NOT_FOUND", [Session, SessionInput], None, false;
    SessionInputAlreadyBound = 3203, "SESSION_INPUT_ALREADY_BOUND", [Session, SessionInput], None, false;
    SessionInputInvalid = 3204, "SESSION_INPUT_INVALID", [Session, SessionInput], None, false;
    SessionOwnershipMismatch = 3205, "SESSION_OWNERSHIP_MISMATCH", [Session], None, false;
    SessionHistoryWriteFailed = 3301, "SESSION_HISTORY_WRITE_FAILED", [Session, SessionHistory], HistoryWrite, false;
    SessionHistoryLoadFailed = 3302, "SESSION_HISTORY_LOAD_FAILED", [Session, SessionHistory], HistoryLoad, false;
    SessionConfigInvalid = 3401, "SESSION_CONFIG_INVALID", [Session, SessionConfig], None, false;
    SessionConfigConflict = 3402, "SESSION_CONFIG_CONFLICT", [Session, SessionConfig], None, false;
    SessionResourceMissing = 3403, "SESSION_RESOURCE_MISSING", [Session, SessionConfig], None, false;
    SessionOutputNotFound = 3501, "SESSION_OUTPUT_NOT_FOUND", [Session, SessionLookup], None, false;
    SessionOutputRangeInvalid = 3502, "SESSION_OUTPUT_RANGE_INVALID", [Session], None, false;
    SessionHistoryCursorInvalid = 3503, "SESSION_HISTORY_CURSOR_INVALID", [Session, SessionHistory], None, false;
    SessionToolCallNotFound = 3504, "SESSION_TOOLCALL_NOT_FOUND", [Session, SessionLookup], None, false;
    SessionInternalUnexpected = 3999, "SESSION_INTERNAL_UNEXPECTED", [Session], None, false;
    TurnModelUpstreamRateLimited = 4201, "TURN_MODEL_UPSTREAM_RATE_LIMITED", [Turn, TurnModel, TurnModelUpstream], Upstream, true;
    TurnModelUpstreamOverloaded = 4202, "TURN_MODEL_UPSTREAM_OVERLOADED", [Turn, TurnModel, TurnModelUpstream], Upstream, true;
    TurnModelUpstreamQuotaExhausted = 4203, "TURN_MODEL_UPSTREAM_QUOTA_EXHAUSTED", [Turn, TurnModel, TurnModelUpstream], Upstream, false;
    TurnModelUpstreamAuthenticationFailed = 4204, "TURN_MODEL_UPSTREAM_AUTHENTICATION_FAILED", [Turn, TurnModel, TurnModelUpstream], Upstream, false;
    TurnModelStreamInterrupted = 4210, "TURN_MODEL_STREAM_INTERRUPTED", [Turn, TurnModel, TurnModelStream], None, false;
    TurnModelUpstreamUnclassified = 4299, "TURN_MODEL_UPSTREAM_UNCLASSIFIED", [Turn, TurnModel, TurnModelUpstream], Upstream, false;
    TurnInternalUnexpected = 4999, "TURN_INTERNAL_UNEXPECTED", [Turn], None, false;
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
    /// Opaque data from an unrecognized leaf. It is retained for the client, not
    /// interpreted as a registered schema or accepted by known leaf builders.
    Unrecognized(serde_json::Value),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FaultIdentity {
    Registered(FaultKind),
    // Keep the common registered Fault small. Unknown definitions are received
    // from a peer, never synthesized by an owner as a fallback classification.
    Unrecognized(Box<UnrecognizedIdentity>),
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct UnrecognizedIdentity {
    type_name: String,
    code: u16,
}

/// Public business fault. Owners construct registered leaves; clients also
/// retain newer unknown leaves without dropping receipts or inventing a known
/// class. Registered type/code/details combinations remain strictly checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fault {
    identity: FaultIdentity,
    message: String,
    diagnostic_id: Option<String>,
    details: Option<FaultDetails>,
    retry_after_ms: Option<u64>,
}
impl Fault {
    /// The owner supplies a safe human-readable summary, not a raw error body.
    pub fn new(kind: FaultKind, message: impl Into<String>) -> Self {
        Self {
            identity: FaultIdentity::Registered(kind),
            message: message.into(),
            diagnostic_id: None,
            details: None,
            retry_after_ms: None,
        }
    }
    /// A known registry leaf, or None when this client does not recognize the
    /// received type. Unknown names never acquire classes from their spelling.
    pub const fn kind(&self) -> Option<FaultKind> {
        match &self.identity {
            FaultIdentity::Registered(kind) => Some(*kind),
            FaultIdentity::Unrecognized(_) => None,
        }
    }
    pub fn type_name(&self) -> &str {
        match &self.identity {
            FaultIdentity::Registered(kind) => kind.type_name(),
            FaultIdentity::Unrecognized(identity) => &identity.type_name,
        }
    }
    pub fn code(&self) -> u16 {
        match &self.identity {
            FaultIdentity::Registered(kind) => kind.code(),
            FaultIdentity::Unrecognized(identity) => identity.code,
        }
    }
    pub fn is_a(&self, class: FaultClass) -> bool {
        self.kind().is_some_and(|kind| kind.is_a(class))
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
        let Some(kind) = self.kind() else {
            if self
                .details
                .as_ref()
                .is_some_and(|details| !matches!(details, FaultDetails::Unrecognized(_)))
            {
                return Err(invalid(
                    "unrecognized faults cannot claim registered details",
                ));
            }
            return Ok(());
        };
        if self.retry_after_ms.is_some() && !kind.allows_retry_after_ms() {
            return Err(invalid(
                "retry_after_ms is only registered for temporary upstream faults",
            ));
        }
        match &self.details {
            None => {}
            Some(FaultDetails::HistoryWrite(_))
                if kind.details_schema() == FaultDetailsSchema::HistoryWrite => {}
            Some(FaultDetails::HistoryLoad(location))
                if kind.details_schema() == FaultDetailsSchema::HistoryLoad =>
            {
                if location.line == Some(0) {
                    return Err(invalid("history location line is one-based"));
                }
            }
            Some(FaultDetails::Upstream(_))
                if kind.details_schema() == FaultDetailsSchema::Upstream => {}
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
            self.type_name(),
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
        state.serialize_field("type", self.type_name())?;
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
    type_name: String,
    code: u16,
    message: String,
    #[serde(default, deserialize_with = "crate::present")]
    diagnostic_id: Option<String>,
    #[serde(default, deserialize_with = "crate::present")]
    details: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "crate::present")]
    retry_after_ms: Option<u64>,
}
impl<'de> Deserialize<'de> for Fault {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = FaultWire::deserialize(d)?;
        let kind = FAULT_KINDS
            .iter()
            .copied()
            .find(|kind| kind.type_name() == wire.type_name);
        let Some(kind) = kind else {
            if wire.type_name.is_empty() || FAULT_KINDS.iter().any(|kind| kind.code() == wire.code)
            {
                return Err(serde::de::Error::custom(
                    "fault type/code registry mismatch",
                ));
            }
            return Ok(Self {
                identity: FaultIdentity::Unrecognized(Box::new(UnrecognizedIdentity {
                    type_name: wire.type_name,
                    code: wire.code,
                })),
                message: wire.message,
                diagnostic_id: wire.diagnostic_id,
                details: wire.details.map(FaultDetails::Unrecognized),
                retry_after_ms: wire.retry_after_ms,
            });
        };
        if wire.code != kind.code() {
            return Err(serde::de::Error::custom(
                "fault type/code registry mismatch",
            ));
        }
        let details = wire
            .details
            .map(|value| {
                match kind.details_schema() {
                    FaultDetailsSchema::HistoryWrite => {
                        serde_json::from_value(value).map(FaultDetails::HistoryWrite)
                    }
                    FaultDetailsSchema::HistoryLoad => {
                        serde_json::from_value(value).map(FaultDetails::HistoryLoad)
                    }
                    FaultDetailsSchema::Upstream => {
                        serde_json::from_value(value).map(FaultDetails::Upstream)
                    }
                    FaultDetailsSchema::None => {
                        return Err(serde::de::Error::custom(
                            "this fault type has no registered details",
                        ));
                    }
                }
                .map_err(serde::de::Error::custom)
            })
            .transpose()?;
        let fault = Self {
            identity: FaultIdentity::Registered(kind),
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
