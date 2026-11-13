//! Session input acceptance and Runtime queue consumption share the owner state.
use super::*;

#[derive(Clone)]
pub(super) struct QueuedInput {
    pub(super) received_at: Timestamp,
    pub(super) message: MessageInput,
    pub(super) source: MessageSource,
    pub(super) runtime: UserInput,
    pub(super) queue: InputQueue,
    pub(super) order: u64,
}
impl QueuedInput {
    pub(super) fn summary(&self) -> InputSummary {
        InputSummary {
            message_id: self.message.message_id.clone(),
            mode: self.message.mode,
            queue: self.queue,
            order: self.order,
        }
    }
}
impl SessionController {
    pub(crate) fn send(
        self: &Arc<Self>,
        caller: &CallerContext,
        message: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        self.ensure_executor()?;
        self.access.ensure_execution_owned()?;
        let runtime = UserInput::text(&message.text)
            .map_err(|_| Fault::new(FaultKind::SessionInputInvalid, "input must not be empty"))?;
        let (receipt, record) = self.access.accept_business(caller, || {
            self.accept_message(message, runtime, caller.source())
        })?;
        record.emit();
        self.wake();
        Ok(receipt)
    }
    /// Used only by an already-accepted direct-owner creation workflow.
    pub(crate) fn send_owned(
        self: &Arc<Self>,
        message: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        self.ensure_executor()?;
        self.access.ensure_execution_owned()?;
        let runtime = UserInput::text(&message.text)
            .map_err(|_| Fault::new(FaultKind::SessionInputInvalid, "input must not be empty"))?;
        let (receipt, record) = self.accept_message(message, runtime, MessageSource::User)?;
        // Creation callers have already left their owner/admission scopes.
        record.emit();
        self.wake();
        Ok(receipt)
    }
    pub(super) fn accept_message(
        self: &Arc<Self>,
        message: MessageInput,
        runtime: UserInput,
        source: MessageSource,
    ) -> Result<(MessageAccepted, Record), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        self.accept_message_locked(&mut state, message, runtime, source)
    }
    pub(super) fn accept_message_locked(
        self: &Arc<Self>,
        state: &mut SessionState,
        message: MessageInput,
        runtime: UserInput,
        source: MessageSource,
    ) -> Result<(MessageAccepted, Record), Fault> {
        Self::open(state)?;
        self.access.ensure_execution_owned()?;
        if state.host_stopping {
            return Err(Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "Session is stopping",
            ));
        }
        if !state.agent_enabled.load(Ordering::SeqCst) {
            return Err(Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "owning Agent is stopped",
            ));
        }
        let queue = if state
            .active
            .as_ref()
            .is_some_and(|active| active.info.state == TurnState::Cancelling)
        {
            InputQueue::NextTurn
        } else {
            message.mode.requested_queue()
        };
        let next_order = match queue {
            InputQueue::NextStep => &mut state.next_step_order,
            InputQueue::NextTurn => &mut state.next_turn_order,
        };
        let order = *next_order;
        *next_order = order.checked_add(1).ok_or_else(|| {
            Fault::new(
                FaultKind::SessionInternalUnexpected,
                "queue order exhausted",
            )
        })?;
        let received_at = Timestamp::now();
        state.info.last_activity_at = state.info.last_activity_at.max(received_at);
        let entry = QueuedInput {
            received_at,
            message: message.clone(),
            source: source.clone(),
            runtime,
            queue,
            order,
        };
        state.messages.push(MessageRecord {
            received_at,
            history_position: None,
            input: message.clone(),
            source,
            queue,
            order,
            binding: MessageBinding::Queued,
        });
        match queue {
            InputQueue::NextStep => state.next_step.push_back(entry),
            InputQueue::NextTurn => state.next_turn.push_back(entry),
        }
        // Register the metadata owner before an accepted input can be cleared
        // by shutdown; acceptance and its timestamp leave no quiescent gap.
        self.checkpoint_activity_locked(state);
        self.publish(state);
        let record = self.observe_message(
            state,
            state.messages.last().expect("accepted message"),
            None,
        );
        let mut facts = MessageAcceptedFacts::new(queue, state.controller_revision);
        facts.load_id = state.load_id.clone();
        facts.start_error = if state.active.is_none() {
            state.block_reason.clone()
        } else {
            None
        };
        Ok((
            MessageAccepted::new(
                self.host_id.clone(),
                self.incarnation.clone(),
                SessionRef {
                    agent_id: self.agent_id.clone(),
                    session_id: self.id.clone(),
                },
                message.message_id,
                facts,
            ),
            record,
        ))
    }
    pub(crate) async fn wait_input_owned(
        &self,
        capability: &crate::TurnCapability,
    ) -> Result<(), Fault> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            capability.check_active()?;
            {
                let state = self.state.lock().expect("Session mutex poisoned");
                Self::open(&state)?;
                if self.schedulable(&state)
                    && !state.next_step.is_empty()
                    && state.active.as_ref().is_some_and(|active| {
                        active.info.id == capability.source().turn.turn_id
                            && active.info.state != TurnState::Cancelling
                    })
                {
                    return Ok(());
                }
            }
            changed.await;
        }
    }
}

pub(super) fn mark_message(state: &mut SessionState, input: &QueuedInput, binding: MessageBinding) {
    if let Some(record) = state.messages.iter_mut().find(|record| {
        record.input.message_id == input.message.message_id
            && record.queue == input.queue
            && record.order == input.order
    }) {
        if matches!(binding, MessageBinding::Removed)
            && let Some(slot) = state.delegation.as_mut()
        {
            slot.record_discarded(&input.message.message_id);
        }
        if let MessageBinding::Bound { turn_id } = &binding
            && let Some(slot) = state.delegation.as_mut()
        {
            slot.record_bound(&input.message.message_id, turn_id);
        }
        record.binding = binding;
    }
}
pub(super) fn clear_inbox(state: &mut SessionState) -> Vec<MessageId> {
    let mut entries = state.next_step.drain(..).collect::<Vec<_>>();
    entries.extend(state.next_turn.drain(..));
    let ids = entries
        .iter()
        .map(|input| input.message.message_id.clone())
        .collect();
    for input in entries {
        mark_message(state, &input, MessageBinding::Removed);
    }
    ids
}
pub(super) fn take_batch(
    state: &mut SessionState,
    new_turn: bool,
    turn_id: &TurnId,
) -> Vec<QueuedInput> {
    let mut batch = state.next_step.drain(..).collect::<Vec<_>>();
    if new_turn && let Some(input) = state.next_turn.pop_front() {
        batch.push(input);
    }
    for input in &batch {
        mark_message(
            state,
            input,
            MessageBinding::Bound {
                turn_id: turn_id.clone(),
            },
        );
    }
    if let Some(active) = state
        .active
        .as_mut()
        .filter(|active| &active.info.id == turn_id)
    {
        active
            .bound_inputs
            .extend(batch.iter().map(|input| input.message.message_id.clone()));
    }
    batch
}

impl SessionController {
    /// The Session authors the native input and provenance together. There is
    /// no later payload matching or second delivery from an execution report.
    pub(super) fn accept_bound_inputs(
        &self,
        state: &mut SessionState,
        batch: Vec<QueuedInput>,
    ) -> Vec<Record> {
        let bindings = batch
            .iter()
            .map(|input| (input.queue, input.order))
            .collect::<Vec<_>>();
        let active = state.active.as_ref().expect("owned Turn");
        let turn_id = active.info.id.clone();
        let loop_id = active.loop_id;
        let first = state.history.len() == active.start_history;
        if !batch.is_empty() && state.info.mode == StorageMode::Persistent {
            state.active.as_mut().expect("owned Turn").info.persistence = PersistenceState::Pending;
        }
        for (index, input) in batch.into_iter().enumerate() {
            let message_id = input.message.message_id.clone();
            let user = minicore_host_runtime::UserHistory {
                loop_id,
                kind: if first && index == 0 {
                    minicore_host_runtime::UserMessageKind::Prompt
                } else {
                    minicore_host_runtime::UserMessageKind::Steering
                },
                input: input.runtime,
            };
            let entry = HistoryEntry::Input {
                received_at: input.received_at,
                turn_id: turn_id.clone(),
                message_id: message_id.clone(),
                source: input.source,
                mode: input.message.mode,
                in_reply_to: input.message.in_reply_to,
                user: user.clone(),
            };
            state.input_provenance.record(state.history.len(), &entry);
            Arc::make_mut(&mut state.history).push(HistoryItem::User(user));
            let position = state
                .readable_history
                .as_ref()
                .expect("loaded history")
                .len() as u64;
            let recorded_at = Timestamp::now();
            let record = HistoryRecord::at(self.id.clone(), position, recorded_at, entry);
            state.info.last_activity_at = state.info.last_activity_at.max(recorded_at);
            state
                .readable_history
                .as_mut()
                .expect("loaded history")
                .push(HistoryRecordData::from_record(&record));
            if let Some(message) = state
                .messages
                .iter_mut()
                .find(|message| message.queue == input.queue && message.order == input.order)
            {
                message.history_position = Some(position);
            }
            state
                .active
                .as_mut()
                .expect("owned Turn")
                .consumed_inputs
                .push(message_id.clone());
            Self::record_delegated_input(state, &turn_id, &message_id);
        }
        let active = state.active.as_ref().expect("owned Turn");
        bindings
            .iter()
            .filter_map(|(queue, order)| {
                state
                    .messages
                    .iter()
                    .find(|message| &message.queue == queue && &message.order == order)
            })
            .map(|message| self.observe_message(state, message, Some(&active.info)))
            .collect()
    }
}
