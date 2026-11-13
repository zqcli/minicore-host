//! One validator for accepted batches and decoded durable history.
use crate::{
    CompactionProjection, FilePosition, HISTORY_SCHEMA_VERSION, HistoryEntry, HistoryProblem,
    HistoryRecord, StorageError,
};
use minicore_host_protocol::{SessionId, TurnId, TurnOutcome};
use minicore_host_runtime::{
    LoopId,
    history::HistoryItem,
    model::{AssistantPart, ModelResponse},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn invalid(position: FilePosition, problem: HistoryProblem) -> StorageError {
    StorageError::InvalidHistory { position, problem }
}
pub(super) fn validate_envelope(
    record: &HistoryRecord,
    session: &SessionId,
    expected: u64,
    location: FilePosition,
) -> Result<(), StorageError> {
    if record.schema_version != HISTORY_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchema {
            found: u64::from(record.schema_version),
            supported: HISTORY_SCHEMA_VERSION,
            position: Some(location),
        });
    }
    if &record.session_id != session {
        return Err(invalid(location, HistoryProblem::WrongSession));
    }
    if record.position < expected {
        return Err(invalid(
            location,
            HistoryProblem::DuplicatePosition {
                expected,
                found: record.position,
            },
        ));
    }
    if record.position > expected {
        return Err(invalid(
            location,
            HistoryProblem::UnexpectedPosition {
                expected,
                found: record.position,
            },
        ));
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct Validation {
    native_items: u64,
    covered_items: u64,
    last_non_result: Option<u64>,
    loops: BTreeMap<LoopId, TurnId>,
    response_next: BTreeMap<TurnId, u32>,
    requests: BTreeSet<(LoopId, u32)>,
    calls: BTreeMap<(TurnId, String), SavedCall>,
    finished: BTreeSet<TurnId>,
    partial_turns: BTreeSet<TurnId>,
    partial_loops: BTreeSet<LoopId>,
}
#[derive(Clone)]
struct SavedCall {
    loop_id: LoopId,
    request_index: u32,
    response_index: u32,
    name: String,
    native_position: u64,
    result_position: Option<u64>,
}
impl Validation {
    pub(super) fn merge(&mut self, delta: Self) {
        self.native_items = delta.native_items;
        self.covered_items = delta.covered_items;
        self.last_non_result = delta.last_non_result;
        self.loops.extend(delta.loops);
        self.response_next.extend(delta.response_next);
        self.requests.extend(delta.requests);
        self.calls.extend(delta.calls);
        self.finished.extend(delta.finished);
        self.partial_turns.extend(delta.partial_turns);
        self.partial_loops.extend(delta.partial_loops);
    }
}
// An overlay owns only this batch's index changes. Failed validation drops the
// overlay without copying or mutating any accumulated historical index.
pub(super) struct Validator<'a> {
    base: &'a Validation,
    delta: Validation,
}
impl<'a> Validator<'a> {
    pub(super) fn new(base: &'a Validation) -> Self {
        Self {
            base,
            delta: Validation {
                native_items: base.native_items,
                covered_items: base.covered_items,
                last_non_result: base.last_non_result,
                ..Validation::default()
            },
        }
    }
    pub(super) fn finish(self) -> Validation {
        self.delta
    }

    pub(super) fn entry(
        &mut self,
        entry: &HistoryEntry,
        position: u64,
    ) -> Result<(), HistoryProblem> {
        let bad = |reason: &str| HistoryProblem::InvalidRecord(reason.to_owned());
        if entry
            .turn_id()
            .is_some_and(|id| self.delta.finished.contains(id) || self.base.finished.contains(id))
        {
            return Err(bad("record follows an already finished Turn"));
        }
        match entry {
            HistoryEntry::Input { turn_id, user, .. } => {
                if user.input.as_text().is_empty() {
                    return Err(bad("empty consumed input"));
                }
                self.loop_owner(user.loop_id, turn_id)?;
            }
            HistoryEntry::Runtime {
                turn_id,
                response_index,
                item,
            } => match item {
                HistoryItem::User(_) => {
                    return Err(bad(
                        "Runtime UserHistory must be an Input with consumed-message provenance",
                    ));
                }
                HistoryItem::Summary(_) => {}
                HistoryItem::Assistant(item) => {
                    ModelResponse::new_with_provider_replay(
                        item.content.clone(),
                        item.finish_reason,
                        item.usage,
                        item.provider_replay.clone(),
                    )
                    .map_err(|e| bad(&e.to_string()))?;
                    self.response(turn_id, item.loop_id, item.request_index, *response_index)?;
                    for call in item.content.iter().filter_map(AssistantPart::as_tool_call) {
                        let key = (turn_id.clone(), call.tool_call_id().as_str().to_owned());
                        if self.delta.calls.contains_key(&key) || self.base.calls.contains_key(&key)
                        {
                            return Err(bad("duplicate ToolCall identity within one Turn"));
                        }
                        self.delta.calls.insert(
                            key,
                            SavedCall {
                                loop_id: item.loop_id,
                                request_index: item.request_index,
                                response_index: *response_index,
                                name: call.name().as_str().to_owned(),
                                native_position: self.delta.native_items,
                                result_position: None,
                            },
                        );
                    }
                }
                HistoryItem::ToolResult(item) => {
                    self.loop_owner(item.loop_id, turn_id)?;
                    let key = (turn_id.clone(), item.call_id.as_str().to_owned());
                    if !self.delta.calls.contains_key(&key) {
                        let old =
                            self.base.calls.get(&key).ok_or_else(|| {
                                bad("ToolResult has no preceding matching ToolCall")
                            })?;
                        self.delta.calls.insert(key.clone(), old.clone());
                    }
                    let saved = self
                        .delta
                        .calls
                        .get_mut(&key)
                        .expect("call exists in the batch overlay");
                    if saved.loop_id != item.loop_id
                        || saved.request_index != item.request_index
                        || saved.response_index != *response_index
                        || saved.name != item.tool_name.as_str()
                    {
                        return Err(bad(
                            "ToolResult association conflicts with its original ToolCall",
                        ));
                    }
                    if saved.result_position.is_some() {
                        return Err(bad("duplicate ToolResult"));
                    }
                    if saved.native_position < self.delta.covered_items {
                        return Err(crate::compaction::split_exchange());
                    }
                    saved.result_position = Some(self.delta.native_items);
                }
            },
            HistoryEntry::Partial {
                turn_id,
                loop_id,
                response_index,
                partial,
            } => {
                self.response(turn_id, *loop_id, partial.request_index, *response_index)?;
                if partial.content.is_empty()
                    && partial.incomplete_tool_calls.is_empty()
                    && partial.usage.is_none()
                    && partial.provider_replay.is_none()
                {
                    return Err(bad("empty model partial"));
                }
                let mut ids = BTreeSet::new();
                let mut indices = BTreeSet::new();
                for part in &partial.content {
                    part.validate().map_err(|e| bad(&e.to_string()))?;
                    if let Some(call) = part.as_tool_call()
                        && (!ids.insert(call.tool_call_id().as_str())
                            || !indices.insert(call.call_index()))
                    {
                        return Err(bad("duplicate call within partial"));
                    }
                }
                let total = partial
                    .content
                    .len()
                    .checked_add(partial.incomplete_tool_calls.len())
                    .ok_or_else(|| bad("partial index overflow"))?;
                let mut slots = BTreeSet::new();
                for call in &partial.incomplete_tool_calls {
                    if call.part_index >= total
                        || !slots.insert(call.part_index)
                        || !ids.insert(call.tool_call_id.as_str())
                        || !indices.insert(call.call_index)
                    {
                        return Err(bad(
                            "invalid or duplicate incomplete call index or identity",
                        ));
                    }
                }
                if indices
                    .iter()
                    .enumerate()
                    .any(|(position, index)| u32::try_from(position).ok() != Some(*index))
                {
                    return Err(bad("non-contiguous partial call indices"));
                }
                let incomplete: BTreeMap<_, _> = partial
                    .incomplete_tool_calls
                    .iter()
                    .map(|c| (c.part_index, c))
                    .collect();
                let mut complete = partial.content.iter();
                let mut next_call = 0;
                for slot in 0..total {
                    let call_index = if let Some(call) = incomplete.get(&slot) {
                        Some(call.call_index)
                    } else {
                        complete
                            .next()
                            .and_then(AssistantPart::as_tool_call)
                            .map(|call| call.call_index())
                    };
                    if let Some(index) = call_index {
                        if index != next_call {
                            return Err(bad(
                                "partial call order conflicts with original content positions",
                            ));
                        }
                        next_call += 1;
                    }
                }
                self.delta.partial_turns.insert(turn_id.clone());
                self.delta.partial_loops.insert(*loop_id);
            }
            HistoryEntry::TurnFinished {
                turn_id,
                outcome,
                requests,
                tool_rounds,
                model_attempts,
                ..
            } => {
                let known = self
                    .delta
                    .response_next
                    .get(turn_id)
                    .or_else(|| self.base.response_next.get(turn_id))
                    .copied()
                    .unwrap_or_default() as usize;
                if *requests < known as u64
                    || model_attempts.len() as u64 != *requests
                    || model_attempts[..known].contains(&0)
                {
                    return Err(bad(
                        "terminal request/attempt counts contradict accepted model responses",
                    ));
                }
                let known_batches = self
                    .base
                    .calls
                    .range((turn_id.clone(), String::new())..)
                    .take_while(|((turn, _), _)| turn == turn_id)
                    .chain(
                        self.delta
                            .calls
                            .range((turn_id.clone(), String::new())..)
                            .take_while(|((turn, _), _)| turn == turn_id),
                    )
                    .filter(|(_, call)| call.result_position.is_some())
                    .map(|(_, call)| call.response_index)
                    .collect::<BTreeSet<_>>()
                    .len() as u64;
                if *tool_rounds < known_batches || *tool_rounds > *requests {
                    return Err(bad(
                        "terminal tool-round count contradicts accepted tool results",
                    ));
                }
                if *outcome == TurnOutcome::Completed
                    && (self.delta.partial_turns.contains(turn_id)
                        || self.base.partial_turns.contains(turn_id))
                {
                    return Err(bad(
                        "completed Turn contains an unfinished terminal model response",
                    ));
                }
                self.delta.finished.insert(turn_id.clone());
            }
            HistoryEntry::Summary { .. } => {}
            HistoryEntry::Compaction {
                source_position,
                covered_items,
                summary,
            } => {
                let projection =
                    CompactionProjection::new(*source_position, *covered_items, summary.clone());
                projection.validate_metadata(
                    position,
                    self.delta.covered_items,
                    self.delta.native_items,
                )?;
                if self
                    .delta
                    .calls
                    .values()
                    .chain(
                        self.base
                            .calls
                            .iter()
                            .filter(|(key, _)| !self.delta.calls.contains_key(*key))
                            .map(|(_, call)| call),
                    )
                    .any(|call| {
                        projection.splits_exchange(
                            call.native_position,
                            call.result_position,
                            self.delta.last_non_result,
                        )
                    })
                {
                    return Err(crate::compaction::split_exchange());
                }
                self.delta.covered_items = *covered_items;
            }
        }
        match entry {
            HistoryEntry::Input { .. }
            | HistoryEntry::Summary { .. }
            | HistoryEntry::Runtime { .. } => {
                if !matches!(
                    entry,
                    HistoryEntry::Runtime {
                        item: HistoryItem::ToolResult(_),
                        ..
                    }
                ) {
                    self.delta.last_non_result = Some(self.delta.native_items);
                }
                self.delta.native_items = self
                    .delta
                    .native_items
                    .checked_add(1)
                    .ok_or_else(|| bad("native history position overflow"))?;
            }
            HistoryEntry::Partial { .. }
            | HistoryEntry::TurnFinished { .. }
            | HistoryEntry::Compaction { .. } => {}
        }
        Ok(())
    }
    fn loop_owner(&mut self, loop_id: LoopId, turn: &TurnId) -> Result<(), HistoryProblem> {
        if self.delta.partial_loops.contains(&loop_id) || self.base.partial_loops.contains(&loop_id)
        {
            return Err(HistoryProblem::InvalidRecord(
                "record follows this Loop's terminal partial".into(),
            ));
        }
        match self
            .delta
            .loops
            .get(&loop_id)
            .or_else(|| self.base.loops.get(&loop_id))
        {
            Some(owner) if owner != turn => Err(HistoryProblem::InvalidRecord(
                "Loop belongs to another Turn".into(),
            )),
            Some(_) => Ok(()),
            None => {
                self.delta.loops.insert(loop_id, turn.clone());
                Ok(())
            }
        }
    }
    fn response(
        &mut self,
        turn: &TurnId,
        loop_id: LoopId,
        request: u32,
        response: u32,
    ) -> Result<(), HistoryProblem> {
        self.loop_owner(loop_id, turn)?;
        let next = self
            .delta
            .response_next
            .get(turn)
            .or_else(|| self.base.response_next.get(turn))
            .copied()
            .unwrap_or_default();
        if response != next {
            return Err(HistoryProblem::InvalidRecord(
                "response_index is not the next contiguous Turn response".into(),
            ));
        }
        let key = (loop_id, request);
        if self.delta.requests.contains(&key) || self.base.requests.contains(&key) {
            return Err(HistoryProblem::InvalidRecord(
                "duplicate Runtime logical response".into(),
            ));
        }
        self.delta.requests.insert(key);
        self.delta.response_next.insert(
            turn.clone(),
            next.checked_add(1)
                .ok_or_else(|| HistoryProblem::InvalidRecord("response index overflow".into()))?,
        );
        Ok(())
    }
}
