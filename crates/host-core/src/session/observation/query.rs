use super::*;
use crate::{
    MessageData, MessageSnapshot, TurnDetailsData, TurnDetailsSnapshot, TurnHistoryItemData,
    TurnListEntry, TurnListSnapshot, TurnPartialData, TurnResultData, TurnResultSnapshot,
};

impl SessionController {
    pub(crate) fn message_snapshot(
        &self,
        caller: &CallerContext,
        id: &MessageId,
    ) -> Result<MessageSnapshot, Fault> {
        let record = self.message(caller, id)?;
        Ok(MessageSnapshot {
            source: self.identity(),
            message_id: record.input.message_id,
            data: MessageData {
                text: record.input.text,
                mode: record.input.mode,
                source: record.source,
                in_reply_to: record.input.in_reply_to,
                queue: record.queue,
                order: record.order,
                binding: record.binding,
            },
        })
    }

    /// Waiting needs only this fact, never a clone of the full bound Role.
    pub(crate) fn turn_query_finished(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<bool, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            visible(&state)?;
            Ok(retained_turn(&state, id)?.state == TurnState::Finished)
        })
    }

    pub(crate) fn turn_details(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<TurnDetailsSnapshot, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            visible(&state)?;
            let info = retained_turn(&state, id)?;
            Ok(TurnDetailsSnapshot {
                source: self.turn_identity(info),
                data: details(info, state.runner_revision),
            })
        })
    }

    pub(crate) fn turn_list(&self, caller: &CallerContext) -> Result<TurnListSnapshot, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            visible(&state)?;
            let data = state
                .completion_order
                .iter()
                .filter_map(|id| state.completed.get(id).map(|result| &result.info))
                .chain(state.active.iter().map(|active| &active.info))
                .map(|info| TurnListEntry {
                    turn_id: info.id.clone(),
                    load_id: info.load_id.clone(),
                    data: details(info, state.runner_revision),
                })
                .collect();
            Ok(TurnListSnapshot {
                source: self.identity(),
                data,
            })
        })
    }

    pub(crate) fn turn_result_snapshot(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<TurnResultSnapshot, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            visible(&state)?;
            let result = state.completed.get(id).ok_or_else(missing_turn)?;
            Ok(TurnResultSnapshot {
                source: self.turn_identity(&result.info),
                data: TurnResultData {
                    info: (&result.info).into(),
                    config: result.info.config.clone(),
                    history: project_history(result, state.output.get(id))?,
                    partials: result
                        .partials
                        .iter()
                        .map(|partial| TurnPartialData {
                            response_index: partial.response_index,
                            partial: (&partial.partial).into(),
                        })
                        .collect(),
                },
            })
        })
    }
}

fn retained_turn<'a>(state: &'a SessionState, id: &TurnId) -> Result<&'a TurnInfo, Fault> {
    state
        .active
        .as_ref()
        .filter(|active| &active.info.id == id)
        .map(|active| &active.info)
        .or_else(|| state.completed.get(id).map(|result| &result.info))
        .ok_or_else(missing_turn)
}
fn details(info: &TurnInfo, revision: u64) -> TurnDetailsData {
    TurnDetailsData {
        state: SessionController::turn_state(info, revision),
        config: info.config.clone(),
    }
}
fn visible(state: &SessionState) -> Result<(), Fault> {
    if state.reclaimed {
        Err(Fault::new(
            FaultKind::AgentSessionNotFound,
            "memory Session has been reclaimed",
        ))
    } else {
        Ok(())
    }
}
fn missing_turn() -> Fault {
    Fault::new(
        FaultKind::SessionTurnNotFound,
        "Turn is not held by this Session",
    )
}
fn missing_response() -> Fault {
    Fault::new(
        FaultKind::SessionInternalUnexpected,
        "retained Turn history has no matching complete response projection",
    )
}

/// Both sources are committed together by `accept_history` under this same
/// Session lock: native complete records and their accepted output positions.
/// Pair complete records in their original order, never count input takes or
/// mistake a per-Loop request index for a public Turn index. The temporary
/// mapping only joins tool records to their original native model response.
fn project_history(
    result: &TurnResult,
    output: Option<&TurnObservation>,
) -> Result<Vec<TurnHistoryItemData>, Fault> {
    let mut indexes = output.into_iter().flat_map(|output| {
        output
            .models
            .iter()
            .filter(|(_, channels)| channels.values().any(|body| body.complete.is_some()))
            .map(|(index, _)| *index)
    });
    let mut responses = BTreeMap::new();
    let mut history = Vec::with_capacity(result.history.len());
    for item in &result.history {
        let response_index = match item {
            HistoryItem::Assistant(value) => {
                let index = indexes.next().ok_or_else(missing_response)?;
                responses.insert((value.loop_id, value.request_index), index);
                Some(index)
            }
            HistoryItem::ToolResult(value) => Some(
                *responses
                    .get(&(value.loop_id, value.request_index))
                    .ok_or_else(missing_response)?,
            ),
            HistoryItem::User(_) | HistoryItem::Summary(_) => None,
        };
        history.push(TurnHistoryItemData {
            response_index,
            item: item.into(),
        });
    }
    if indexes.next().is_some() {
        return Err(missing_response());
    }
    Ok(history)
}

#[cfg(test)]
mod tests;
