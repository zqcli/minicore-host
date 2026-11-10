//! Session input acceptance and Runtime queue consumption share the owner state.
use super::*;

#[derive(Clone)]
pub(super) struct QueuedInput {
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
        &self,
        message: MessageInput,
        runtime: UserInput,
        source: MessageSource,
    ) -> Result<(MessageAccepted, Record), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        self.accept_message_locked(&mut state, message, runtime, source)
    }
    pub(super) fn accept_message_locked(
        &self,
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
        let entry = QueuedInput {
            message: message.clone(),
            source: source.clone(),
            runtime,
            queue,
            order,
        };
        state.messages.push(MessageRecord {
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

pub(super) struct SessionInputs {
    pub(super) session: Arc<SessionController>,
    pub(super) turn_id: TurnId,
    pub(super) initial: Mutex<VecDeque<QueuedInput>>,
}
impl LoopInputs for SessionInputs {
    fn take(&self) -> Vec<UserInput> {
        let mut state = self.session.state.lock().expect("Session mutex poisoned");
        if state.host_stopping || !self.session.access.execution_allowed_owned() {
            let mut claimed = false;
            let handle = state
                .active
                .as_mut()
                .filter(|active| active.info.id == self.turn_id)
                .and_then(|active| {
                    if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                        claimed = true;
                        active._activation.expire();
                        active.info.state = TurnState::Cancelling;
                    }
                    active.handle.clone()
                });
            let record =
                self.session
                    .cancel_claim_record(&state, claimed, "host_force", None, false);
            drop(state);
            diagnostic::emit(record);
            if let Some(handle) = handle {
                handle.shutdown();
            }
            return Vec::new();
        }
        // Already bound remainder is handoff storage, not a third inbox. Pause
        // never withdraws it. Force does not consume even a bound remainder.
        let mut values = self
            .initial
            .lock()
            .expect("bound input mutex poisoned")
            .drain(..)
            .map(|input| input.runtime)
            .collect::<Vec<_>>();
        let mut records = Vec::new();
        if self.session.schedulable(&state)
            && state.active.as_ref().is_some_and(|active| {
                active.info.id == self.turn_id && active.info.state == TurnState::Running
            })
        {
            let batch = take_batch(&mut state, false, &self.turn_id);
            records = self.session.observe_bound(&state, &batch);
            values.extend(batch.iter().map(|input| input.runtime.clone()));
            state
                .active
                .as_mut()
                .expect("checked active Turn")
                .bound_pending
                .extend(batch);
            self.session.publish(&mut state);
        }
        let records = records
            .into_iter()
            .map(|record| record.published(state.controller_revision, state.runner_revision))
            .collect::<Vec<_>>();
        drop(state);
        diagnostic::emit(records);
        values
    }
}
