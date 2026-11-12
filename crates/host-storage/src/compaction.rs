use crate::{HistoryEntry, HistoryProblem};
use minicore_host_runtime::{
    history::{HistoryItem, SummaryHistory},
    model::AssistantPart,
};
use std::collections::BTreeMap;

/// The latest committed model-context replacement. Coverage always counts raw
/// native items, never JSONL rows, partials, finish records or earlier markers.
/// This is derived context, not another persisted file or execution state.
#[derive(Clone, Debug, PartialEq)]
pub struct CompactionProjection {
    pub source_position: u64,
    pub covered_items: u64,
    pub summary: SummaryHistory,
}
impl CompactionProjection {
    pub fn new(source_position: u64, covered_items: u64, summary: SummaryHistory) -> Self {
        Self {
            source_position,
            covered_items,
            summary,
        }
    }

    /// Construct the one append-only record. The journal validates it against
    /// actual prior records; construction alone does not confirm a commit.
    pub fn into_entry(self) -> HistoryEntry {
        HistoryEntry::Compaction {
            source_position: self.source_position,
            covered_items: self.covered_items,
            summary: self.summary,
        }
    }

    /// Pure candidate validation for the Session owner's already validated raw
    /// transcript. The owner separately checks work identity, cancellation and
    /// save outcome.
    /// Historical missing results stay unknown; no tool result is manufactured.
    pub fn validate(
        &self,
        raw_history: &[HistoryItem],
        expected_source_position: u64,
        previous_coverage: u64,
    ) -> Result<(), HistoryProblem> {
        self.validate_metadata(
            expected_source_position,
            previous_coverage,
            raw_history.len() as u64,
        )?;
        let covered = self.covered_items as usize;
        if matches!(raw_history.get(covered), Some(HistoryItem::ToolResult(_))) {
            return Err(split_exchange());
        }
        // Match actual Runtime associations, not globally unique call strings.
        // Reusing the same ToolCallId in another request/Loop is legal.
        let mut calls = BTreeMap::new();
        let mut last_non_result = None;
        for (index, item) in raw_history.iter().enumerate() {
            if !matches!(item, HistoryItem::ToolResult(_)) {
                last_non_result = Some(index as u64);
            }
            match item {
                HistoryItem::Assistant(assistant) => {
                    for call in assistant
                        .content
                        .iter()
                        .filter_map(AssistantPart::as_tool_call)
                    {
                        let key = (
                            assistant.loop_id,
                            assistant.request_index,
                            call.tool_call_id().as_str(),
                            call.name().as_str(),
                        );
                        if calls.insert(key, (index as u64, None)).is_some() {
                            return Err(bad("duplicate native ToolCall association"));
                        }
                    }
                }
                HistoryItem::ToolResult(result) => {
                    let key = (
                        result.loop_id,
                        result.request_index,
                        result.call_id.as_str(),
                        result.tool_name.as_str(),
                    );
                    let Some((_, result_position)) = calls.get_mut(&key) else {
                        return Err(bad("native ToolResult has no preceding matching ToolCall"));
                    };
                    if result_position.replace(index as u64).is_some() {
                        return Err(bad("duplicate native ToolResult"));
                    }
                }
                HistoryItem::User(_) | HistoryItem::Summary(_) => {}
            }
        }
        if calls
            .values()
            .any(|(call, result)| self.splits_exchange(*call, *result, last_non_result))
        {
            return Err(split_exchange());
        }
        Ok(())
    }

    /// Latest committed summary followed by the untouched raw suffix. This does
    /// not mutate or normalize either the transcript or actual tool results.
    pub fn project(&self, raw_history: &[HistoryItem]) -> Result<Vec<HistoryItem>, HistoryProblem> {
        self.validate(raw_history, self.source_position, 0)?;
        let mut projected = Vec::with_capacity(raw_history.len() - self.covered_items as usize + 1);
        projected.push(HistoryItem::Summary(self.summary.clone()));
        projected.extend_from_slice(&raw_history[self.covered_items as usize..]);
        Ok(projected)
    }

    pub(crate) fn splits_exchange(
        &self,
        call: u64,
        result: Option<u64>,
        last_non_result: Option<u64>,
    ) -> bool {
        call < self.covered_items
            && (result.is_some_and(|result| result >= self.covered_items)
                || (result.is_none() && Some(call) == last_non_result))
    }

    pub(crate) fn validate_metadata(
        &self,
        expected_source_position: u64,
        previous_coverage: u64,
        native_items: u64,
    ) -> Result<(), HistoryProblem> {
        if self.source_position != expected_source_position {
            return Err(bad(
                "compaction source does not match its Session-local position",
            ));
        }
        if self.covered_items == 0 || self.covered_items <= previous_coverage {
            return Err(bad(
                "compaction coverage must advance the previous raw prefix",
            ));
        }
        if self.covered_items > native_items {
            return Err(bad("compaction coverage exceeds preceding native history"));
        }
        if self.summary.content.as_str().trim().is_empty() {
            return Err(bad("compaction summary is blank"));
        }
        Ok(())
    }
}

pub(crate) fn split_exchange() -> HistoryProblem {
    bad("compaction coverage splits a native tool exchange")
}
fn bad(reason: &str) -> HistoryProblem {
    HistoryProblem::InvalidRecord(reason.to_owned())
}
