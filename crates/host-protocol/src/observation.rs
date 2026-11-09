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
        if self.channel.is_empty()
            || self.end.checked_sub(self.start) != Some(self.text.len() as u64)
        {
            return Err(invalid(
                "output range must match its UTF-8 byte length and name a channel",
            ));
        }
        Ok(())
    }
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
        OutputDelta {
            channel: self.channel.clone(),
            start: self.start,
            end: self.end,
            text: self.text.clone(),
        }
        .validate()?;
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
}
