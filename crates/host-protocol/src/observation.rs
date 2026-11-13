//! Shared observation scopes, source identities and UTF-8 range contracts.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    State,
    Output,
    Completion,
    Message,
    Interaction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubscriptionScope {
    Host {
        host_id: HostId,
    },
    Agent {
        host_id: HostId,
        agent_id: AgentId,
    },
    Session {
        agent_id: AgentId,
        session_id: SessionId,
    },
    Turn {
        session_id: SessionId,
        turn_id: TurnId,
    },
}
validated_dto!(SubscribeRequest, SubscribeRequestWire, "SubscribeRequest" {
    scope: SubscriptionScope,
    categories: BTreeSet<EventCategory>,
});
impl SubscribeRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.categories.is_empty() {
            return Err(invalid("categories must not be empty"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsubscribeRequest {
    pub subscription_id: SubscriptionId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsubscribeResult {
    pub ended: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeResult<S> {
    pub subscription_id: SubscriptionId,
    pub initial_states: Vec<S>,
    pub output_positions: Vec<OutputPosition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionPush<E> {
    pub subscription_id: SubscriptionId,
    pub event: E,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResyncRequired {
    pub subscription_id: SubscriptionId,
    pub fault: Fault,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionIdentity {
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub agent_id: AgentId,
    pub session_id: SessionId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TurnIdentity {
    #[serde(flatten)]
    pub session: SessionIdentity,
    pub turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_id: Option<LoadId>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelOutputIdentity {
    #[serde(flatten)]
    pub turn: TurnIdentity,
    pub response_index: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolCallIdentity {
    #[serde(flatten)]
    pub turn: TurnIdentity,
    pub tool_call_id: ToolCallId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputSource {
    Model {
        #[serde(flatten)]
        source: ModelOutputIdentity,
    },
    Toolcall {
        #[serde(flatten)]
        source: ToolCallIdentity,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutputPosition {
    #[serde(flatten)]
    pub source: OutputSource,
    pub channel: String,
    pub end: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelPosition {
    pub channel: String,
    pub end: u64,
}

validated_dto!(OutputDelta, OutputDeltaWire, "OutputDelta" {
    channel: String, start: u64, end: u64, text: String,
});
impl OutputDelta {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_output_range(&self.channel, self.start, self.end, &self.text)
    }
}
fn validate_output_range(
    channel: &str,
    start: u64,
    end: u64,
    text: &str,
) -> Result<(), ValidationError> {
    if channel.is_empty() || end.checked_sub(start) != Some(text.len() as u64) {
        return Err(invalid(
            "output range must match its UTF-8 byte length and name a channel",
        ));
    }
    Ok(())
}
validated_dto!(
/// `end` is the fixed catch-up boundary; growth after it is not part of this read.
/// `max_bytes` is a caller-selected page size, not a source retention quota.
TextReadRequest, TextReadRequestWire, "TextReadRequest" {
    channel: String, start: u64, end: u64, max_bytes: u64,
});
impl TextReadRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.channel.is_empty() || self.start > self.end || self.max_bytes == 0 {
            return Err(invalid(
                "text read requires a channel, ordered range and positive page size",
            ));
        }
        Ok(())
    }
}
validated_dto!(TextRangeData, TextRangeDataWire, "TextRangeData" {
    channel: String, start: u64, end: u64, text: String, has_more: bool,
});
impl TextRangeData {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_output_range(&self.channel, self.start, self.end, &self.text)?;
        if self.has_more && self.start == self.end {
            return Err(invalid("non-tail reads must advance"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryPageData<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    /// Exclusive native-record boundary captured for the entire pagination window.
    pub upper_bound: u64,
}

/// Owner-captured wall time. Comparison uses the instant, never an RFC3339 string.
/// Serialization always normalizes the instant to UTC.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(time::OffsetDateTime);
impl Timestamp {
    pub fn now() -> Self {
        Self(time::OffsetDateTime::now_utc())
    }
    pub fn from_unix_milliseconds(value: i64) -> Result<Self, ValidationError> {
        let value = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000_000)
            .map_err(|_| invalid("timestamp is outside the supported range"))?;
        Self::normalized(value)
    }
    fn normalized(value: time::OffsetDateTime) -> Result<Self, ValidationError> {
        value
            .checked_to_offset(time::UtcOffset::UTC)
            .filter(|value| value.year() >= 0)
            .map(Self)
            .ok_or_else(|| invalid("timestamp cannot be represented as UTC RFC3339"))
    }
}
impl Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        time::serde::rfc3339::serialize(&self.0, serializer)
    }
}
impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        time::serde::rfc3339::deserialize(deserializer)
            .and_then(|value| Self::normalized(value).map_err(serde::de::Error::custom))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryDirection {
    #[default]
    Forward,
    Backward,
}

validated_dto!(HistoryReadRequest, HistoryReadRequestWire, "HistoryReadRequest" {
    cursor: Option<String>, limit: u64,
    #[serde(default)]
    direction: HistoryDirection,
});
impl HistoryReadRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.limit == 0 {
            return Err(invalid("history page limit must be positive"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    Model,
    Tools,
    WaitingForInput,
    Finalizing,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelRetryPhase {
    Waiting {
        #[serde(with = "time::serde::rfc3339")]
        retry_at: time::OffsetDateTime,
    },
    Requesting,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRetryProgress {
    pub response_index: u64,
    pub attempt: u8,
    pub max_attempts: u8,
    pub fault: Fault,
    pub phase: ModelRetryPhase,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum RetryProgressWire {
    Waiting {
        response_index: u64,
        attempt: u8,
        max_attempts: u8,
        fault: Fault,
        #[serde(with = "time::serde::rfc3339")]
        retry_at: time::OffsetDateTime,
    },
    Requesting {
        response_index: u64,
        attempt: u8,
        max_attempts: u8,
        fault: Fault,
    },
}
impl Serialize for ModelRetryProgress {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        match self.phase {
            ModelRetryPhase::Waiting { retry_at } => RetryProgressWire::Waiting {
                response_index: self.response_index,
                attempt: self.attempt,
                max_attempts: self.max_attempts,
                fault: self.fault.clone(),
                retry_at,
            },
            ModelRetryPhase::Requesting => RetryProgressWire::Requesting {
                response_index: self.response_index,
                attempt: self.attempt,
                max_attempts: self.max_attempts,
                fault: self.fault.clone(),
            },
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for ModelRetryProgress {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = match RetryProgressWire::deserialize(deserializer)? {
            RetryProgressWire::Waiting {
                response_index,
                attempt,
                max_attempts,
                fault,
                retry_at,
            } => Self {
                response_index,
                attempt,
                max_attempts,
                fault,
                phase: ModelRetryPhase::Waiting { retry_at },
            },
            RetryProgressWire::Requesting {
                response_index,
                attempt,
                max_attempts,
                fault,
            } => Self {
                response_index,
                attempt,
                max_attempts,
                fault,
                phase: ModelRetryPhase::Requesting,
            },
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
impl ModelRetryProgress {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.attempt < 2 || self.attempt > self.max_attempts {
            return Err(invalid(
                "retry attempts include the initial request and start at two",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod history_contract_tests {
    use super::*;

    #[test]
    fn timestamps_compare_instants_and_serialize_in_utc() {
        let offset: Timestamp = serde_json::from_str("\"2026-11-13T12:00:00.1+02:00\"").unwrap();
        let utc: Timestamp = serde_json::from_str("\"2026-11-13T10:00:00.100Z\"").unwrap();
        let later: Timestamp = serde_json::from_str("\"2026-11-13T10:00:00.11Z\"").unwrap();
        assert_eq!(offset, utc);
        assert!(offset < later);
        assert_eq!(
            serde_json::to_string(&offset).unwrap(),
            "\"2026-11-13T10:00:00.1Z\""
        );
        for invalid in [
            "2026-13-40T10:00:00Z",
            "9999-12-31T23:59:59-23:59",
            "0000-01-01T00:00:00+01:00",
        ] {
            assert!(serde_json::from_value::<Timestamp>(serde_json::json!(invalid)).is_err());
        }
        assert!(Timestamp::from_unix_milliseconds(-62_167_219_200_001).is_err());
    }

    #[test]
    fn history_direction_defaults_forward_and_limit_still_validates() {
        let request: HistoryReadRequest =
            serde_json::from_str(r#"{"cursor":null,"limit":10}"#).unwrap();
        assert_eq!(request.direction, HistoryDirection::Forward);
        assert!(
            serde_json::from_str::<HistoryReadRequest>(
                r#"{"cursor":null,"limit":0,"direction":"backward"}"#
            )
            .is_err()
        );
    }
}
