use serde::{Deserialize, Serialize};

use crate::execution::UserInput;
use crate::ids::{LoopId, ToolCallId};
use crate::model::{
    AssistantPart, ModelFinishReason, ModelRef, ProviderReplay, ReasoningPreference, Usage,
};
use crate::tools::{ToolName, ToolOutput, ToolResultOutcome};
use crate::value::Text;

/// Native model-context records owned and persisted by the Session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum HistoryItem {
    User(UserHistory),
    Assistant(AssistantHistory),
    ToolResult(ToolResultHistory),
    Summary(SummaryHistory),
}

/// Input accepted by the Session under its execution correlation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserHistory {
    pub loop_id: LoopId,
    pub kind: UserMessageKind,
    pub input: UserInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserMessageKind {
    /// The initial input for the Turn.
    Prompt,
    /// Additional input accepted at a request boundary.
    Steering,
}

/// A complete, locally validated model response for one request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantHistory {
    pub loop_id: LoopId,
    pub request_index: u32,
    pub model: ModelRef,
    pub reasoning: ReasoningPreference,
    /// Reuses the existing typed assistant parts; no second part vocabulary.
    pub content: Vec<AssistantPart>,
    /// Provider-owned metadata. Never used for display or tool execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_replay: Option<ProviderReplay>,
    pub finish_reason: ModelFinishReason,
    pub usage: Usage,
}

/// The result of one tool call under its original execution correlation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultHistory {
    pub loop_id: LoopId,
    pub request_index: u32,
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    pub outcome: ToolResultOutcome,
    pub output: ToolOutput,
}

/// Host-managed summary without durable boundary semantics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SummaryHistory {
    pub content: Text,
}

/// Borrowed view of shared historical records and the newly appended suffix.
#[derive(Clone, Copy, Debug)]
pub struct HistoryView<'a> {
    base: &'a [HistoryItem],
    appended: &'a [HistoryItem],
}

impl<'a> HistoryView<'a> {
    pub fn new(base: &'a [HistoryItem], appended: &'a [HistoryItem]) -> Self {
        Self { base, appended }
    }

    pub fn base(&self) -> &'a [HistoryItem] {
        self.base
    }

    pub fn appended(&self) -> &'a [HistoryItem] {
        self.appended
    }

    pub fn len(&self) -> usize {
        self.base.len() + self.appended.len()
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.appended.is_empty()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &'a HistoryItem> {
        self.base.iter().chain(self.appended.iter())
    }
}
