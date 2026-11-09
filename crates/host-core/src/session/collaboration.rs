use super::*;
use crate::collaboration::*;
use std::collections::BTreeSet;

pub(super) struct DelegationSlot {
    relation: Arc<DelegationState>,
    sink: Arc<dyn DelegationSink>,
    accepted: BTreeSet<MessageId>,
    first_bound_turn: Option<TurnId>,
    handed_off: bool,
    results: Vec<TurnResult>,
    inputs: Vec<DelegationInputRange>,
    discarded: Vec<MessageId>,
}
impl SessionController {
    pub(crate) fn wake_owned(self: &Arc<Self>) {
        self.wake();
    }
    pub(crate) fn register_scope_owned(
        self: &Arc<Self>,
        hooks: Arc<dyn ScopeHooks>,
    ) -> Result<ScopePermit, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        Self::open(&state)?;
        if state.delegation.is_some() || state.root_scope.is_some() {
            return Err(Fault::new(
                FaultKind::SessionOwnershipMismatch,
                "Session already belongs to a collaboration scope",
            ));
        }
        let scope = Arc::new(ScopeState {
            root: Arc::downgrade(self),
            root_ref: SessionRef {
                agent_id: self.agent_id.clone(),
                session_id: self.id.clone(),
            },
            access: Arc::downgrade(&self.access),
            hooks,
            sealed: AtomicBool::new(false),
            cleanup: tokio::sync::OnceCell::new(),
            pending: std::sync::atomic::AtomicUsize::new(0),
            notified_closed: AtomicBool::new(false),
        });
        state.root_scope = Some(Arc::clone(&scope));
        Ok(ScopePermit { state: scope })
    }
    pub(crate) fn scope_owned(&self, root: &SessionRef) -> Result<Arc<ScopeState>, Fault> {
        let state = self.state.lock().expect("Session mutex poisoned");
        Self::open(&state)?;
        let scope = state
            .root_scope
            .as_ref()
            .cloned()
            .or_else(|| {
                state
                    .delegation
                    .as_ref()
                    .map(|slot| Arc::clone(&slot.relation.scope))
            })
            .filter(|scope| &scope.root_ref == root)
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionOwnershipMismatch,
                    "bound source does not belong to this collaboration scope",
                )
            })?;
        scope.check_open()?;
        Ok(scope)
    }
    pub(crate) fn seal_scope_owned(&self, scope: &Arc<ScopeState>) -> Result<(), Fault> {
        self.access.accept_internal_owned(|| {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::available(&state)?;
            if !state
                .root_scope
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, scope))
            {
                return Err(Fault::new(
                    FaultKind::SessionOwnershipMismatch,
                    "scope is not held by the original root",
                ));
            }
            scope.sealed.store(true, Ordering::SeqCst);
            self.publish(&mut state);
            Ok(())
        })?;
        scope.hooks.seal();
        Ok(())
    }
    pub(crate) fn register_delegation_owned(
        self: &Arc<Self>,
        registration: DelegationRegistration,
        parent: Weak<SessionController>,
        scope: Arc<ScopeState>,
        sink: Arc<dyn DelegationSink>,
    ) -> Result<DelegationPermit, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        Self::open(&state)?;
        self.access.ensure_business_owned()?;
        if state.root_scope.is_some() || state.delegation.is_some() {
            return Err(Fault::new(
                FaultKind::SessionOwnershipMismatch,
                "target already has a registered collaboration relationship",
            ));
        }
        let relation = Arc::new(DelegationState {
            registration,
            child: Arc::downgrade(self),
            parent,
            scope,
            access: Arc::downgrade(&self.access),
            notified: AtomicBool::new(false),
            handed_off: AtomicBool::new(false),
            released: AtomicBool::new(false),
            close: tokio::sync::OnceCell::new(),
        });
        relation.scope.pending.fetch_add(1, Ordering::SeqCst);
        state.delegation = Some(DelegationSlot {
            relation: Arc::clone(&relation),
            sink,
            accepted: BTreeSet::new(),
            first_bound_turn: None,
            handed_off: false,
            results: Vec::new(),
            inputs: Vec::new(),
            discarded: Vec::new(),
        });
        Ok(DelegationPermit { state: relation })
    }
    pub(crate) fn receive_delegated_owned(
        &self,
        registration: &DelegationRegistration,
        scope: &Arc<ScopeState>,
        source: SessionRef,
        message: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        let runtime = UserInput::text(&message.text)
            .map_err(|_| Fault::new(FaultKind::SessionInputInvalid, "input must not be empty"))?;
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let slot = state
            .delegation
            .as_ref()
            .filter(|slot| {
                slot.relation.registration == *registration
                    && Arc::ptr_eq(&slot.relation.scope, scope)
            })
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionOwnershipMismatch,
                    "target has no matching active delegation registration",
                )
            })?;
        if slot.handed_off || slot.relation.released.load(Ordering::SeqCst) {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "delegation result has already been handed off",
            ));
        }
        if slot.accepted.is_empty() {
            self.access.ensure_business_owned()?;
            if message.message_id != registration.task_message_id {
                return Err(Fault::new(
                    FaultKind::SessionInputInvalid,
                    "first delegated input must match its registered task MessageId",
                ));
            }
        }
        let id = message.message_id.clone();
        let receipt = self.accept_message_locked(
            &mut state,
            message,
            runtime,
            MessageSource::Agent { source },
        )?;
        state
            .delegation
            .as_mut()
            .expect("registered slot")
            .accepted
            .insert(id);
        Ok(receipt)
    }
    pub(crate) fn receive_notification_owned(
        &self,
        relation: &DelegationState,
        mut message: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        relation.scope.check_open()?;
        message.mode = if state.active.is_some() {
            MessageMode::Steer
        } else {
            MessageMode::FollowUp
        };
        message.in_reply_to = Some(relation.registration.task_message_id.clone());
        let runtime = UserInput::text(&message.text).map_err(|_| {
            Fault::new(
                FaultKind::SessionInputInvalid,
                "notification must not be empty",
            )
        })?;
        self.accept_message_locked(
            &mut state,
            message,
            runtime,
            MessageSource::Agent {
                source: relation.registration.child.clone(),
            },
        )
    }
    pub(crate) fn release_delegation_owned(
        &self,
        relation: &Arc<DelegationState>,
        disposition: DelegationDisposition,
    ) -> Result<(), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let Some(slot) = state
            .delegation
            .as_ref()
            .filter(|slot| Arc::ptr_eq(&slot.relation, relation))
        else {
            return Ok(());
        };
        let valid = match disposition {
            DelegationDisposition::NotAccepted => slot.accepted.is_empty(),
            DelegationDisposition::Notified(_) | DelegationDisposition::NotificationFailed(_) => {
                slot.handed_off
            }
            DelegationDisposition::ScopeClosed => {
                relation.scope.sealed.load(Ordering::SeqCst)
                    && state.active.is_none()
                    && !state.saving
                    && state.history_work.is_none()
            }
        };
        if !valid || state.handoff_pending {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "delegation still owns accepted input or necessary result handoff",
            ));
        }
        state.delegation = None;
        self.publish(&mut state);
        Ok(())
    }
    pub(super) fn record_delegated_input(
        state: &mut SessionState,
        turn: &TurnId,
        message: &MessageId,
    ) {
        if let Some(slot) = state.delegation.as_mut() {
            slot.record_bound(message, turn);
        }
    }
    pub(super) fn record_delegated_result(
        state: &mut SessionState,
        result: &TurnResult,
        consumed_inputs: Vec<MessageId>,
    ) {
        if let Some(slot) = state.delegation.as_mut()
            && slot.first_bound_turn.is_some()
            && !slot.handed_off
        {
            slot.inputs.push(DelegationInputRange {
                turn: TurnRef {
                    session_id: state.info.id.clone(),
                    turn_id: result.info.id.clone(),
                },
                delegated_message_ids: consumed_inputs
                    .iter()
                    .filter(|id| slot.accepted.contains(*id))
                    .cloned()
                    .collect(),
                message_ids: consumed_inputs,
            });
            slot.results.push(result.clone());
        }
    }
    pub(crate) fn handoff_delegation_owned(&self) {
        let prepared = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.active.is_some()
                || state.saving
                || state.history_work.is_some()
                || state.record_work.is_some()
                || state.preparing
                || state.handoff_pending
                || !state.next_step.is_empty()
                || !state.next_turn.is_empty()
            {
                return;
            }
            let Some(slot) = state
                .delegation
                .as_mut()
                .filter(|slot| slot.needs_handoff())
            else {
                return;
            };
            let prepared = (
                Arc::clone(&slot.sink),
                DelegationCompletion {
                    results: std::mem::take(&mut slot.results),
                    inputs: std::mem::take(&mut slot.inputs),
                    discarded_inputs: std::mem::take(&mut slot.discarded),
                },
            );
            slot.handed_off = true;
            state.handoff_pending = true;
            prepared
        };
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| prepared.0.handoff(prepared.1)))
            .unwrap_or_else(|_| {
                Err(Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "delegation result sink panicked",
                ))
            });
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.handoff_pending = false;
        match result {
            Ok(()) => {
                if let Some(slot) = &state.delegation {
                    slot.relation.handed_off.store(true, Ordering::SeqCst);
                }
            }
            Err(fault) => state.close_fault = Some(fault),
        }
        Self::finish_memory_close(&mut state);
        self.publish(&mut state);
    }
    pub(crate) fn record_delegation_owned(
        &self,
        origin: &ToolCallRef,
        registration: &DelegationRegistration,
    ) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.reclaimed {
            return;
        }
        let revision = state.runner_revision;
        let load_id = state
            .active
            .as_ref()
            .filter(|active| active.info.id == origin.turn_id)
            .map(|active| active.info.load_id.clone())
            .or_else(|| {
                state
                    .completed
                    .get(&origin.turn_id)
                    .map(|result| result.info.load_id.clone())
            });
        let Some(record) = state
            .output
            .get_mut(&origin.turn_id)
            .and_then(|turn| turn.tools.get_mut(&origin.tool_call_id))
        else {
            return;
        };
        if !record.data.state.delegations.iter().any(|item| {
            item.child == registration.child && item.task_message_id == registration.task_message_id
        }) {
            record
                .data
                .state
                .delegations
                .push(crate::DelegationAssociation {
                    child: registration.child.clone(),
                    task_message_id: registration.task_message_id.clone(),
                });
        }
        record.data.state.runner_revision = revision;
        self.access
            .observation
            .publish(crate::ObservationEvent::State(
                crate::StateEvent::Toolcall {
                    source: ToolCallIdentity {
                        turn: TurnIdentity {
                            session: self.identity(),
                            turn_id: origin.turn_id.clone(),
                            load_id,
                        },
                        tool_call_id: origin.tool_call_id.clone(),
                    },
                    data: record.data.state.clone(),
                },
            ));
    }
}
impl DelegationSlot {
    pub(super) fn record_bound(&mut self, id: &MessageId, turn: &TurnId) {
        // Either queue can supply the first real input for this relationship.
        // Retain the shared activity, not an exclusive result for its first message.
        if self.accepted.contains(id) {
            self.first_bound_turn.get_or_insert_with(|| turn.clone());
        }
    }
    pub(super) fn record_discarded(&mut self, id: &MessageId) {
        if self.accepted.contains(id) {
            self.discarded.push(id.clone());
        }
    }
    pub(super) fn scope_closed(&self) -> bool {
        self.relation.scope.sealed.load(Ordering::SeqCst)
    }
    pub(super) fn needs_handoff(&self) -> bool {
        (self.first_bound_turn.is_some()
            || self
                .discarded
                .contains(&self.relation.registration.task_message_id))
            && !self.handed_off
    }
    pub(super) fn pending(&self) -> bool {
        !self.accepted.is_empty()
            && !self.handed_off
            && !self.relation.scope.sealed.load(Ordering::SeqCst)
    }
}
impl SessionController {
    pub(super) fn start_scope_cleanup_owned(&self) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        self.start_scope_cleanup_locked(&mut state, true);
    }
    pub(super) fn start_scope_cleanup_locked(&self, state: &mut SessionState, explicit: bool) {
        let Some(scope) = state.root_scope.clone() else {
            return;
        };
        scope.sealed.store(true, Ordering::SeqCst);
        if state.scope_cleanup_pending
            || scope.cleanup.get().is_some()
            || (state.scope_task.is_some() && !explicit)
        {
            return;
        }
        let Some(owner) = scope.root.upgrade() else {
            return;
        };
        let Some(runtime) = state
            .executor
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        else {
            return;
        };
        state.scope_cleanup_pending = true;
        let previous = state.scope_task.take();
        state.scope_task = Some(runtime.spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = AssertUnwindSafe(scope.cleanup())
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(Fault::new(
                        FaultKind::SessionInternalUnexpected,
                        "scope cleanup owner panicked",
                    ))
                });
            {
                let mut state = owner.state.lock().expect("Session mutex poisoned");
                state.scope_cleanup_pending = false;
                state.close_fault = result.err();
                Self::finish_memory_close(&mut state);
                owner.publish(&mut state);
            }
            owner.scope_root_closed_owned();
        }));
        self.publish(state);
    }
    pub(super) fn scope_root_closed_owned(&self) {
        let scope = {
            let state = self.state.lock().expect("Session mutex poisoned");
            (state.lifecycle == SessionLifecycle::Closed || state.released)
                .then(|| state.root_scope.clone())
                .flatten()
        };
        if let Some(scope) = scope {
            scope.root_closed();
        }
    }
    pub(crate) async fn join_close_owned(&self) -> Result<(), Fault> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.state.lock().expect("Session mutex poisoned");
                if let Some(fault) = &state.close_fault {
                    return Err(fault.clone());
                }
                if state.released || state.reclaimed {
                    return Ok(());
                }
                if state.lifecycle == SessionLifecycle::Closed
                    && !state.driver_running
                    && !state.handoff_pending
                    && !state.scope_cleanup_pending
                    && state.history_work.is_none()
                    && state.record_work.is_none()
                {
                    return Ok(());
                }
                if let Some(fault) = state
                    .history_work
                    .as_ref()
                    .and_then(|work| work.fault.clone())
                {
                    return Err(fault);
                }
            }
            changed.await;
        }
    }
}

impl SessionController {
    pub(crate) async fn wait_handoff_owned(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self
                .state
                .lock()
                .expect("Session mutex poisoned")
                .handoff_pending
            {
                return;
            }
            changed.await;
        }
    }
    pub(crate) fn signal_collaboration_owned(&self) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        Self::finish_memory_close(&mut state);
        self.publish(&mut state);
    }
}

impl SessionController {
    pub(super) fn finish_memory_close(state: &mut SessionState) {
        if state.lifecycle == SessionLifecycle::Closing
            && state.info.mode == StorageMode::Memory
            && state.active.is_none()
            && state.compaction.is_none()
            && !state.preparing
            && !state.saving
            && state.history_work.is_none()
            && state.record_work.is_none()
            && !state.handoff_pending
            && !state.scope_cleanup_pending
            && state.close_fault.is_none()
            && !state
                .delegation
                .as_ref()
                .is_some_and(DelegationSlot::needs_handoff)
            && state
                .root_scope
                .as_ref()
                .is_none_or(|scope| scope.pending.load(Ordering::SeqCst) == 0)
        {
            state.lifecycle = SessionLifecycle::Closed;
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
    }
}

impl SessionController {
    pub(crate) fn retired_owned(&self) -> bool {
        let state = self.state.lock().expect("Session mutex poisoned");
        state.released || state.reclaimed
    }
    pub(super) fn resume_scope_cleanup(&self, state: &mut SessionState) -> bool {
        if state.close_fault.is_some()
            && !state.scope_cleanup_pending
            && state.scope_task.is_some()
            && state.root_scope.as_ref().is_some_and(|scope| {
                scope.sealed.load(Ordering::SeqCst) && scope.cleanup.get().is_none()
            })
        {
            self.start_scope_cleanup_locked(state, true);
            true
        } else {
            false
        }
    }
}
