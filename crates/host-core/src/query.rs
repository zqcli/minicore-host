//! Public query values. Source owners retain the underlying data; these values
//! neither reconstruct previous executions nor add a second retention store.
use crate::{HistoryItemData, MessageBinding, ModelPartialData, TurnStateData};
use minicore_host_protocol::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageData {
    pub received_at: Timestamp,
    pub history_position: Option<u64>,
    pub text: String,
    pub mode: MessageMode,
    pub source: MessageSource,
    pub in_reply_to: Option<MessageId>,
    pub queue: InputQueue,
    pub order: u64,
    pub binding: MessageBinding,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageSnapshot {
    #[serde(flatten)]
    pub source: SessionIdentity,
    pub message_id: MessageId,
    pub data: MessageData,
}

/// Detailed Turn reads add the original binding to the same lightweight state
/// payload used by observations. State events never carry this configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnDetailsData {
    #[serde(flatten)]
    pub state: TurnStateData,
    pub config: SessionConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnDetailsSnapshot {
    #[serde(flatten)]
    pub source: TurnIdentity,
    pub data: TurnDetailsData,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnListEntry {
    pub turn_id: TurnId,
    pub load_id: LoadId,
    #[serde(flatten)]
    pub data: TurnDetailsData,
}
/// Completion order followed by the active Turn, if any. The actual Session
/// identity appears once, even when the existing result set is empty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnListSnapshot {
    #[serde(flatten)]
    pub source: SessionIdentity,
    pub data: Vec<TurnListEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnHistoryItemData {
    pub position: u64,
    pub recorded_at: Timestamp,
    /// The actual public Turn index for complete model and tool records. User
    /// input and summaries do not belong to a model response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_index: Option<u64>,
    #[serde(flatten)]
    pub item: HistoryItemData,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnPartialData {
    pub position: u64,
    pub recorded_at: Timestamp,
    pub response_index: u64,
    pub partial: ModelPartialData,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "TurnResultDataWire")]
pub struct TurnResultData {
    /// Original current-run metadata, shared with get/list and observations.
    /// Absent for saved-only results: the journal does not retain these fields.
    pub info: Option<TurnStateData>,
    pub config: Option<SessionConfig>,
    /// The exact durable terminal fact, only when reading a saved-only result.
    /// A saved reason is not a reconstructed structured Fault.
    pub saved_terminal: Option<SavedTurnTerminal>,
    /// Complete native records only. Unfinished responses remain in `partials`.
    pub history: Vec<TurnHistoryItemData>,
    pub partials: Vec<TurnPartialData>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedTurnTerminal {
    pub position: u64,
    pub recorded_at: Timestamp,
    #[serde(flatten)]
    pub terminal: TurnTerminalData,
}
impl std::ops::Deref for SavedTurnTerminal {
    type Target = TurnTerminalData;
    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnTerminalData {
    pub finished_at: Timestamp,
    pub requests: u64,
    pub tool_rounds: u64,
    pub model_attempts: Vec<u8>,
    pub outcome: TurnOutcome,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "TurnResultSnapshotWire")]
pub struct TurnResultSnapshot {
    pub source: TurnIdentity,
    pub data: TurnResultData,
}

#[derive(Deserialize)]
struct TurnResultDataWire {
    info: Option<TurnStateData>,
    config: Option<SessionConfig>,
    saved_terminal: Option<SavedTurnTerminal>,
    history: Vec<TurnHistoryItemData>,
    partials: Vec<TurnPartialData>,
}
impl TryFrom<TurnResultDataWire> for TurnResultData {
    type Error = &'static str;

    fn try_from(value: TurnResultDataWire) -> Result<Self, Self::Error> {
        let result = Self {
            info: value.info,
            config: value.config,
            saved_terminal: value.saved_terminal,
            history: value.history,
            partials: value.partials,
        };
        result.validate_metadata()?;
        Ok(result)
    }
}
impl TurnResultData {
    fn validate_metadata(&self) -> Result<(), &'static str> {
        match (&self.info, &self.config, &self.saved_terminal) {
            (Some(_), Some(_), None) | (None, None, Some(_)) => Ok(()),
            _ => Err("Turn result requires either retained info/config or saved_terminal"),
        }
    }
}
impl Serialize for TurnResultData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        self.validate_metadata()
            .map_err(serde::ser::Error::custom)?;
        let mut output = serializer
            .serialize_struct("TurnResultData", if self.info.is_some() { 4 } else { 3 })?;
        if let Some(info) = &self.info {
            output.serialize_field("info", info)?;
        }
        if let Some(config) = &self.config {
            output.serialize_field("config", config)?;
        }
        if let Some(terminal) = &self.saved_terminal {
            output.serialize_field("saved_terminal", terminal)?;
        }
        output.serialize_field("history", &self.history)?;
        output.serialize_field("partials", &self.partials)?;
        output.end()
    }
}

#[derive(Deserialize)]
struct TurnResultSnapshotWire {
    #[serde(flatten)]
    source: TurnIdentity,
    data: TurnResultData,
}
impl TryFrom<TurnResultSnapshotWire> for TurnResultSnapshot {
    type Error = &'static str;

    fn try_from(value: TurnResultSnapshotWire) -> Result<Self, Self::Error> {
        let result = Self {
            source: value.source,
            data: value.data,
        };
        result.validate_source()?;
        Ok(result)
    }
}
impl TurnResultSnapshot {
    fn validate_source(&self) -> Result<(), &'static str> {
        if self.source.load_id.is_some() != self.data.info.is_some() {
            Err("Turn result load_id belongs only to retained execution metadata")
        } else {
            Ok(())
        }
    }
}
impl Serialize for TurnResultSnapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate_source().map_err(serde::ser::Error::custom)?;
        #[derive(Serialize)]
        struct Borrowed<'a> {
            #[serde(flatten)]
            source: &'a TurnIdentity,
            data: &'a TurnResultData,
        }
        Borrowed {
            source: &self.source,
            data: &self.data,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests;
