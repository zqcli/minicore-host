mod history;
mod output;
mod saved;

use super::observation::TurnObservation;
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
    ) -> Result<Option<bool>, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            visible(&state)?;
            Ok(state
                .active
                .as_ref()
                .filter(|active| &active.info.id == id)
                .map(|active| &active.info)
                .or_else(|| state.completed.get(id).map(|result| &result.info))
                .map(|info| info.state == TurnState::Finished))
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
                data: details(&state, info),
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
                    data: details(&state, info),
                })
                .collect();
            Ok(TurnListSnapshot {
                source: self.identity(),
                data,
            })
        })
    }

    fn retained_result_snapshot(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<Option<TurnResultSnapshot>, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            visible(&state)?;
            let Some(result) = state.completed.get(id) else {
                // An actual current Turn, including finalizing/unconfirmed work,
                // cannot be upgraded to finished by a stale or readable disk fact.
                if state
                    .active
                    .as_ref()
                    .is_some_and(|active| &active.info.id == id)
                    || state.info.mode == StorageMode::Memory
                {
                    return Err(missing_turn());
                }
                Self::available(&state)?;
                return Ok(None);
            };
            Ok(Some(TurnResultSnapshot {
                source: self.turn_identity(&result.info),
                data: TurnResultData {
                    info: Some(Self::retained_turn_state(&state, &result.info)),
                    config: Some(result.info.config.clone()),
                    saved_terminal: None,
                    history: project_history(
                        result,
                        state.output.get(id),
                        &state.input_provenance,
                    )?,
                    partials: result
                        .partials
                        .iter()
                        .map(|partial| TurnPartialData {
                            response_index: partial.response_index,
                            partial: (&partial.partial).into(),
                        })
                        .collect(),
                },
            }))
        })
    }
    pub(crate) fn info(&self, caller: &CallerContext) -> Result<SessionInfo, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            Ok(self
                .state
                .lock()
                .expect("Session mutex poisoned")
                .info
                .clone())
        })
    }
    pub(crate) fn snapshot(&self, caller: &CallerContext) -> Result<SessionSnapshot, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            Ok(self.snapshots.borrow().clone())
        })
    }
    pub(crate) fn message(
        &self,
        caller: &CallerContext,
        id: &MessageId,
    ) -> Result<MessageRecord, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            self.state
                .lock()
                .expect("Session mutex poisoned")
                .messages
                .iter()
                .rev()
                .find(|message| &message.input.message_id == id)
                .cloned()
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionInputNotFound,
                        "message is not held by this Session",
                    )
                })
        })
    }
    pub(crate) fn history(&self, caller: &CallerContext) -> Result<Vec<HistoryItem>, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            if !state.history_loaded {
                return Err(Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "history has not yet been loaded and validated",
                ));
            }
            let history = Arc::clone(&state.history);
            drop(state);
            Ok(history.as_ref().clone())
        })
    }

    pub(crate) fn turn_info(&self, caller: &CallerContext, id: &TurnId) -> Result<TurnInfo, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            state
                .active
                .as_ref()
                .filter(|active| &active.info.id == id)
                .map(|active| active.info.clone())
                .or_else(|| state.completed.get(id).map(|result| result.info.clone()))
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionTurnNotFound,
                        "Turn is not held by this Session",
                    )
                })
        })
    }
    pub(crate) fn turns(&self, caller: &CallerContext) -> Result<Vec<TurnInfo>, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            let mut turns = state
                .completion_order
                .iter()
                .filter_map(|id| state.completed.get(id).map(|result| result.info.clone()))
                .collect::<Vec<_>>();
            if let Some(active) = &state.active {
                turns.push(active.info.clone());
            }
            Ok(turns)
        })
    }
    pub(crate) fn turn_result(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            self.state
                .lock()
                .expect("Session mutex poisoned")
                .completed
                .get(id)
                .map(|held| held.result.clone())
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionTurnNotFound,
                        "completed Turn result is not held by this Session",
                    )
                })
        })
    }
    #[cfg(test)]
    pub(crate) async fn wait_turn(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let info = self.turn_info(caller, id)?;
            if info.state == TurnState::Finished {
                return self.turn_result(caller, id);
            }
            notified.await;
        }
    }
    pub(crate) fn input_list(&self, caller: &CallerContext) -> Result<InputList, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            let entry = |queued: &QueuedInput| QueuedInputRecord {
                input: queued.message.clone(),
                source: queued.source.clone(),
                queue: queued.queue,
                order: queued.order,
            };
            Ok(InputList {
                host_id: self.host_id.clone(),
                host_incarnation: self.incarnation.clone(),
                agent_id: self.agent_id.clone(),
                session_id: self.id.clone(),
                controller_revision: state.controller_revision,
                next_step: state.next_step.iter().map(entry).collect(),
                next_turn: state.next_turn.iter().map(entry).collect(),
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
fn details(state: &SessionState, info: &TurnInfo) -> TurnDetailsData {
    TurnDetailsData {
        state: SessionController::retained_turn_state(state, info),
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
        "Turn is not available in this Session",
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
    result: &RetainedTurnResult,
    output: Option<&TurnObservation>,
    input_provenance: &input_provenance::InputProvenance,
) -> Result<Vec<TurnHistoryItemData>, Fault> {
    let mut indexes = output.into_iter().flat_map(|output| {
        output
            .models
            .iter()
            .filter(|(_, channels)| channels.values().any(|body| body.completed))
            .map(|(index, _)| *index)
    });
    let mut responses = BTreeMap::new();
    let mut history = Vec::with_capacity(result.history.len());
    for (offset, item) in result.history.iter().enumerate() {
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
            item: input_provenance.history_item(result.start_history + offset, item),
        });
    }
    if indexes.next().is_some() {
        return Err(missing_response());
    }
    Ok(history)
}

#[cfg(test)]
mod tests;
