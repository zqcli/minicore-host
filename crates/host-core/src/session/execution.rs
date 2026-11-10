//! Public Turn scheduling and Runtime orchestration owned by this Session.
use super::history::SessionHistory;
use super::input::SessionInputs;
use super::*;

impl SessionController {
    pub(super) async fn prepare_resources(
        &self,
        config: &SessionConfig,
        binding: crate::PreparedTurnBinding,
    ) -> Result<BoundResources, Fault> {
        let resources = self.resources_owned()?;
        let preparation = resources.resolve_for_turn(config, binding);
        tokio::pin!(preparation);
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.state.lock().expect("Session mutex poisoned");
                if state.deleting {
                    return Err(Fault::new(
                        FaultKind::SessionLifecycleClosing,
                        "Session deletion fenced unfinished Turn preparation",
                    ));
                }
                if state.host_stopping || !self.access.execution_allowed_owned() {
                    return Err(Fault::new(
                        FaultKind::HostLifecycleUnavailable,
                        "Host stopped before resource preparation completed",
                    ));
                }
            }
            tokio::select! {
                result = &mut preparation => return result,
                _ = changed => {}
            }
        }
    }

    pub(super) fn ensure_executor(&self) -> Result<(), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.executor.is_none() {
            state.executor = tokio::runtime::Handle::try_current().ok();
        }
        if state.executor.is_none() {
            return Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "execution requires a live Tokio runtime before input can be accepted",
            ));
        }
        Ok(())
    }
    pub(crate) fn executor(&self) -> Option<tokio::runtime::Handle> {
        self.state
            .lock()
            .expect("Session mutex poisoned")
            .executor
            .clone()
    }
    pub(super) fn wake(self: &Arc<Self>) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.driver_running
            || state.preparing
            || !self.schedulable(&state)
            || (state.next_step.is_empty() && state.next_turn.is_empty())
        {
            return;
        }
        let Some(executor) = state
            .executor
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        else {
            return;
        };
        state.executor = Some(executor.clone());
        state.driver_running = true;
        let owner = Arc::clone(self);
        let previous = state.driver_task.take();
        state.driver_task = Some(executor.spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            owner.drive().await;
        }));
    }
    pub(super) async fn drive(self: Arc<Self>) {
        loop {
            let preparation = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                if !self.schedulable(&state)
                    || state.preparing
                    || state.active.is_some()
                    || (state.next_step.is_empty() && state.next_turn.is_empty())
                {
                    let previous = state.lifecycle;
                    Self::finish_memory_close(&mut state);
                    state.driver_running = false;
                    self.publish(&mut state);
                    let closed = self.closed_record(&state, previous);
                    drop(state);
                    diagnostic::emit(closed);
                    self.handoff_delegation_owned();
                    return;
                }
                state.preparing = true;
                let candidate = (state.info.config.clone(), state.info.config_revision);
                self.publish(&mut state);
                candidate
            };
            let needs_load = !self
                .state
                .lock()
                .expect("Session mutex poisoned")
                .history_loaded;
            if needs_load {
                {
                    let mut state = self.state.lock().expect("Session mutex poisoned");
                    state.residency = Residency::Loading;
                    self.publish(&mut state);
                }
                match AssertUnwindSafe(self.backend.load())
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Err(Fault::new(
                            FaultKind::SessionHistoryLoadFailed,
                            "history loader failed unexpectedly",
                        ))
                    }) {
                    Ok(history) => {
                        let mut state = self.state.lock().expect("Session mutex poisoned");
                        if let Err(fault) = state.replace_history(&history) {
                            drop(state);
                            self.preparation_failed(fault);
                            return;
                        }
                        state.history_loaded = true;
                        // Validated history is Controller-owned; no Runner has
                        // been bound yet. Do not leave a completed load stuck
                        // in loading if pause/cancel prevents Turn binding.
                        state.residency = Residency::Unloaded;
                        state.load_id = None;
                        self.finish_load_resume(&mut state);
                        self.publish(&mut state);
                    }
                    Err(fault) => {
                        self.preparation_failed(fault);
                        return;
                    }
                }
            }
            let turn_id = match TurnId::new() {
                Ok(id) => id,
                Err(error) => {
                    self.preparation_failed(Fault::new(
                        FaultKind::SessionInternalUnexpected,
                        error.to_string(),
                    ));
                    return;
                }
            };
            let binding = crate::PreparedTurnBinding::new(
                &self,
                crate::TurnSource {
                    host_id: self.host_id.clone(),
                    host_incarnation: self.incarnation.clone(),
                    session: SessionRef {
                        agent_id: self.agent_id.clone(),
                        session_id: self.id.clone(),
                    },
                    turn: TurnRef {
                        session_id: self.id.clone(),
                        turn_id: turn_id.clone(),
                    },
                },
                &self.access,
            );
            let resources =
                AssertUnwindSafe(self.prepare_resources(&preparation.0, binding.clone()))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Err(Fault::new(
                            FaultKind::SessionInternalUnexpected,
                            "resource preparation panicked; accepted inputs remain queued",
                        ))
                    });
            let resources = resources.and_then(|mut resources| {
                resources.execution =
                    self.bind_compaction_prompt(&turn_id, &preparation.0, &resources.execution)?;
                Ok(resources)
            });
            let resources = match resources {
                Ok(resources) => resources,
                Err(fault) => {
                    let mut state = self.state.lock().expect("Session mutex poisoned");
                    state.preparing = false;
                    if state.info.config_revision != preparation.1 {
                        continue;
                    }
                    let schedule = diagnostics::Schedule::capture(&state);
                    state.block_reason = Some(fault);
                    state.driver_running = false;
                    let previous = state.lifecycle;
                    Self::finish_memory_close(&mut state);
                    self.publish(&mut state);
                    let refused = self.resource_refused(
                        &state,
                        state
                            .block_reason
                            .as_ref()
                            .expect("actual preparation failure"),
                    );
                    let scheduling =
                        self.scheduling_record(&state, schedule, "resource_prepare_failed", None);
                    let closed = self.closed_record(&state, previous);
                    drop(state);
                    refused.emit();
                    diagnostic::emit(scheduling);
                    diagnostic::emit(closed);
                    self.handoff_delegation_owned();
                    return;
                }
            };
            let context_binding = crate::context::BoundContextConfiguration::new(
                &preparation.0,
                resources.execution.model().descriptor(),
            );
            let load_id = match LoadId::new() {
                Ok(id) => id,
                Err(error) => {
                    self.preparation_failed(Fault::new(
                        FaultKind::SessionInternalUnexpected,
                        error.to_string(),
                    ));
                    return;
                }
            };
            let (batch, records, context) = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                state.preparing = false;
                if !self.schedulable(&state) || state.info.config_revision != preparation.1 {
                    self.publish(&mut state);
                    continue;
                }
                let batch = take_batch(&mut state, true, &turn_id);
                if batch.is_empty() {
                    continue;
                }
                let load_id = state.load_id.get_or_insert(load_id).clone();
                state.residency = Residency::Loaded;
                let start_history = state.history.len();
                let (activation, cleanup) = binding.activate();
                state.active = Some(ActiveTurn {
                    started_at: Instant::now(),
                    _activation: activation,
                    tool_cleanup: tool_cleanup::ToolCleanupWork::new(cleanup),
                    context_binding,
                    info: TurnInfo {
                        id: turn_id.clone(),
                        session_id: self.id.clone(),
                        load_id,
                        state: TurnState::Starting,
                        config_revision: preparation.1,
                        config: preparation.0.clone(),
                        outcome: None,
                        persistence: if state.info.mode == StorageMode::Memory {
                            PersistenceState::NotApplicable
                        } else {
                            PersistenceState::Pending
                        },
                        requests: 0,
                        tool_rounds: 0,
                        phase: None,
                        model_retry: None,
                        model_attempts: Vec::new(),
                        fault: None,
                        persistence_fault: None,
                        cleanup_faults: Vec::new(),
                    },
                    loop_id: None,
                    handle: None,
                    accepted: 0,
                    consumed_inputs: Vec::new(),
                    bound_inputs: batch
                        .iter()
                        .map(|input| input.message.message_id.clone())
                        .collect(),
                    response_base: 0,
                    bound_pending: VecDeque::new(),
                    start_history,
                    raw_base_len: start_history,
                    partials: Vec::new(),
                    report: None,
                    compaction_failure: None,
                    partial_accepted: false,
                    terminal_accepted: false,
                    pending_interaction: None,
                });
                let records = self.observe_bound(&state, &batch);
                self.publish(&mut state);
                let mut records = records
                    .into_iter()
                    .map(|record| {
                        record.published(state.controller_revision, state.runner_revision)
                    })
                    .collect::<Vec<_>>();
                let active = state.active.as_ref().expect("bound actual Turn");
                records.push(diagnostics::turn_started(
                    &self.turn_identity(&active.info),
                    &active.info,
                    state.controller_revision,
                ));
                let context = diagnostic::turn_context(&self.turn_identity(&active.info));
                (batch, records, context)
            };
            diagnostic::emit(records);
            self.run_turn(turn_id, batch, resources)
                .instrument(context.span())
                .await;
        }
    }
    pub(super) fn preparation_failed(&self, fault: Fault) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let schedule = diagnostics::Schedule::capture(&state);
        let previous = state.lifecycle;
        let record = (fault.kind() == Some(FaultKind::SessionHistoryLoadFailed)).then(|| {
            diagnostics::history_failed(
                diagnostic::session_context(&self.identity()),
                &fault,
                "prepare_load",
                state.controller_revision,
            )
        });
        state.preparing = false;
        state.resume_after_load = None;
        if fault.kind() == Some(FaultKind::SessionHistoryLoadFailed) {
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
        state.block_reason = Some(fault);
        state.driver_running = false;
        Self::finish_memory_close(&mut state);
        self.publish(&mut state);
        let record =
            record.map(|record| record.published(state.controller_revision, state.runner_revision));
        let scheduling = self.scheduling_record(&state, schedule, "preparation_failed", None);
        let closed = self.closed_record(&state, previous);
        drop(state);
        diagnostic::emit(record);
        diagnostic::emit(scheduling);
        diagnostic::emit(closed);
        self.handoff_delegation_owned();
    }
    pub(super) async fn run_turn(
        self: &Arc<Self>,
        turn_id: TurnId,
        mut batch: Vec<QueuedInput>,
        resources: BoundResources,
    ) {
        let resolver = match self.resources_owned() {
            Ok(resolver) => resolver,
            Err(fault) => {
                self.finalize(&turn_id, TurnOutcome::Failed, Some(fault))
                    .await;
                return;
            }
        };
        let execution = resolver.bind_tool_output(
            &resources.execution,
            ToolOutputBinding::new(self, turn_id.clone()),
        );
        let execution = match self.bind_turn_model(&turn_id, &execution) {
            Ok(execution) => execution,
            Err(fault) => {
                self.finalize(&turn_id, TurnOutcome::Failed, Some(fault))
                    .await;
                return;
            }
        };
        loop {
            let started = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let Some(active) = state
                    .active
                    .as_ref()
                    .filter(|active| active.info.id == turn_id)
                else {
                    return;
                };
                if active.info.state == TurnState::Cancelling
                    || state.host_stopping
                    || !self.access.execution_allowed_owned()
                {
                    None
                } else {
                    let history: Arc<[HistoryItem]> = state.history.clone().into();
                    let first = batch.remove(0);
                    let inputs = Arc::new(SessionInputs {
                        session: Arc::clone(self),
                        turn_id: turn_id.clone(),
                        initial: Mutex::new(batch.iter().cloned().collect()),
                    });
                    let callback = Arc::new(SessionHistory {
                        session: Arc::clone(self),
                        turn_id: turn_id.clone(),
                    });
                    let mut options = resources.options.clone();
                    options.history = Some(callback);
                    let raw_base_len = state.history.len();
                    let active = state.active.as_mut().expect("checked active Turn");
                    active.raw_base_len = raw_base_len;
                    active.report = None;
                    active.partial_accepted = false;
                    active.bound_pending.push_back(first.clone());
                    active.bound_pending.extend(batch.drain(..));
                    active.accepted = 0;
                    let started = AgentLoop::start(
                        LoopRequest::new(history, first.runtime, execution.clone()),
                        options,
                        inputs,
                    );
                    match &started {
                        Ok(owner) => {
                            active.loop_id = Some(owner.id());
                            active.handle = Some(owner.handle());
                            active.info.state = TurnState::Running;
                            state.runner_revision = state
                                .runner_revision
                                .checked_add(1)
                                .expect("runner revision exhausted");
                        }
                        Err(_) => {
                            active._activation.expire();
                            active.info.state = TurnState::Finalizing;
                        }
                    }
                    self.publish(&mut state);
                    Some(started)
                }
            };
            let Some(started) = started else {
                self.finalize(&turn_id, TurnOutcome::Cancelled, None).await;
                return;
            };
            let report = match started {
                Ok(mut owner) => {
                    let mut events = owner
                        .take_events()
                        .expect("Session owns the Runtime event receiver");
                    let loop_id = owner.id();
                    let mut runtime_state = owner.handle().watch_state();
                    let joined = owner.join();
                    tokio::pin!(joined);
                    let joined = loop {
                        tokio::select! {
                            result = &mut joined => {
                                while let Some(envelope) = events.recv().await {
                                    self.observe_runtime(&turn_id, loop_id, envelope.event);
                                }
                                self.observe_runtime_state(&turn_id, loop_id, runtime_state.borrow_and_update().clone());
                                break result;
                            }
                            Some(envelope) = events.recv() => self.observe_runtime(&turn_id, loop_id, envelope.event),
                            Ok(()) = runtime_state.changed() => self.observe_runtime_state(&turn_id, loop_id, runtime_state.borrow_and_update().clone()),
                        }
                    };
                    match joined {
                        Ok(report) => report,
                        Err(error) => {
                            self.finalize(
                                &turn_id,
                                TurnOutcome::Failed,
                                Some(Fault::new(
                                    FaultKind::TurnInternalUnexpected,
                                    error.to_string(),
                                )),
                            )
                            .await;
                            return;
                        }
                    }
                }
                Err(error) => {
                    self.finalize(
                        &turn_id,
                        TurnOutcome::Failed,
                        Some(Fault::new(
                            FaultKind::TurnInternalUnexpected,
                            error.to_string(),
                        )),
                    )
                    .await;
                    return;
                }
            };
            let (record, scheduling, source, response_base) = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let previous = diagnostics::Schedule::capture(&state);
                let active = state.active.as_mut().expect("owned active Turn");
                let source = self.turn_identity(&active.info);
                let response_base = active.response_base;
                active.info.requests += u64::from(report.requests);
                active.info.tool_rounds += report.tool_rounds;
                for (index, attempts) in report.model_attempts.iter().enumerate() {
                    let index = active.response_base as usize + index;
                    active
                        .info
                        .model_attempts
                        .resize(active.info.model_attempts.len().max(index + 1), 0);
                    active.info.model_attempts[index] = *attempts;
                }
                active.info.model_retry = None;
                active.report = Some(Arc::clone(&report));
                active.handle = None;
                // Failed/cancelled execution cannot become cancellable again
                // while this owner saves its returned report or partial.
                let record = if report.outcome != LoopOutcome::Completed {
                    let (newly_claimed, forwarded) = active.claim_report(&report);
                    if active.info.outcome == Some(TurnOutcome::Failed) {
                        state.failure_paused = true;
                    }
                    self.publish(&mut state);
                    self.terminal_claim_record(&state, newly_claimed && !forwarded)
                } else {
                    None
                };
                let scheduling = self.scheduling_record(&state, previous, "execution_failed", None);
                (record, scheduling, source, response_base)
            };
            diagnostic::emit(record);
            diagnostic::emit(scheduling);
            diagnostic::emit(diagnostics::models_reported(
                &source,
                response_base,
                &report,
            ));
            if let Err(fault) = self.reconcile_report(&turn_id, &report).await {
                // Ownership of all accepted records remains with this Session.
                // A failed save blocks new work; no reappend or automatic retry.
                let (record, scheduling) = {
                    let mut state = self.state.lock().expect("Session mutex poisoned");
                    let previous = diagnostics::Schedule::capture(&state);
                    state.block_reason = Some(fault.clone());
                    let mut claim = (false, false);
                    if let Some(active) = state.active.as_mut() {
                        active.info.persistence = PersistenceState::Failed;
                        active.info.persistence_fault = Some(fault);
                        claim = active.claim_report(&report);
                        if active.info.outcome == Some(TurnOutcome::Failed) {
                            state.failure_paused = true;
                        }
                    }
                    self.publish(&mut state);
                    (
                        self.terminal_claim_record(&state, claim.0 && !claim.1),
                        self.scheduling_record(&state, previous, "execution_failed", None),
                    )
                };
                diagnostic::emit(record);
                diagnostic::emit(scheduling);
                self.finish_tool_cleanup(&turn_id).await;
                return;
            }
            let (next, records) = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let schedulable = self.schedulable(&state);
                let active = state.active.as_mut().expect("owned active Turn");
                active.handle = None;
                active.response_base += u64::from(report.requests);
                let continue_same = report.outcome == LoopOutcome::Completed
                    && active.info.state != TurnState::Cancelling
                    && schedulable
                    && !state.next_step.is_empty();
                if continue_same {
                    let next = take_batch(&mut state, false, &turn_id);
                    let records = self.observe_bound(&state, &next);
                    self.publish(&mut state);
                    let records = records
                        .into_iter()
                        .map(|record| {
                            record.published(state.controller_revision, state.runner_revision)
                        })
                        .collect::<Vec<_>>();
                    (Some(next), records)
                } else {
                    let active = state.active.as_mut().expect("owned active Turn");
                    let (newly_claimed, forwarded) = active.claim_report(&report);
                    self.publish(&mut state);
                    (
                        None,
                        self.terminal_claim_record(&state, newly_claimed && !forwarded)
                            .into_iter()
                            .collect(),
                    )
                }
            };
            diagnostic::emit(records);
            if let Some(next) = next {
                batch = next;
                continue;
            }
            let (outcome, fault) = {
                let state = self.state.lock().expect("Session mutex poisoned");
                let active = state.active.as_ref().expect("owned active Turn");
                (
                    active.info.outcome.expect("terminal claim"),
                    active.info.fault.clone(),
                )
            };
            self.finalize(&turn_id, outcome, fault).await;
            return;
        }
    }
    pub(super) async fn finalize(
        &self,
        turn_id: &TurnId,
        outcome: TurnOutcome,
        fault: Option<Fault>,
    ) {
        let (record, scheduling) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let previous = diagnostics::Schedule::capture(&state);
            let Some(active) = state
                .active
                .as_mut()
                .filter(|active| &active.info.id == turn_id)
            else {
                return;
            };
            let newly_claimed = active.claim_terminal(outcome, fault);
            if active.info.outcome == Some(TurnOutcome::Failed) {
                state.failure_paused = true;
            }
            self.publish(&mut state);
            (
                self.terminal_claim_record(&state, newly_claimed),
                self.scheduling_record(&state, previous, "finalizing", None),
            )
        };
        diagnostic::emit(record);
        diagnostic::emit(scheduling);
        // Runtime may have dropped a tool future on cancellation. Actual tool
        // workers must still be joined, even if an original history save failed.
        if !self.finish_tool_cleanup(turn_id).await {
            return;
        }
        if !self.save_terminal(turn_id).await {
            return;
        }
        self.finish_pending_answers(turn_id).await;
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.history_work.is_some()
            || state.active.as_ref().is_none_or(|active| {
                &active.info.id != turn_id
                    || !active.tool_cleanup.complete
                    || !active.terminal_accepted
                    || !matches!(
                        active.info.persistence,
                        PersistenceState::Persisted | PersistenceState::NotApplicable
                    )
            })
        {
            return;
        }
        let mut active = state.active.take().expect("checked original Turn");
        active.info.state = TurnState::Finished;
        let result = TurnResult {
            info: active.info,
            history: state.history[active.start_history..].to_vec(),
            partials: active.partials,
        };
        self.observe_finished(&mut state, &result.info, active.report.as_deref());
        let record = diagnostics::turn_finished(
            &self.turn_identity(&result.info),
            &result.info,
            active.started_at,
        );
        state.completion_order.push(turn_id.clone());
        let mut inputs = active.consumed_inputs;
        for id in active.bound_inputs {
            if !inputs.contains(&id) {
                inputs.push(id);
            }
        }
        Self::record_delegated_result(&mut state, &result, inputs);
        state.completed.insert(
            turn_id.clone(),
            RetainedTurnResult {
                result,
                start_history: active.start_history,
            },
        );
        let previous = state.lifecycle;
        Self::finish_memory_close(&mut state);
        self.publish(&mut state);
        let record = record.published(state.controller_revision, state.runner_revision);
        let closed = self.closed_record(&state, previous);
        drop(state);
        record.emit();
        diagnostic::emit(closed);
        self.handoff_delegation_owned();
    }
    pub(crate) fn wake_owned(self: &Arc<Self>) {
        self.wake();
    }
}

pub(super) fn runtime_fault(failure: &minicore_runtime::LoopFailure) -> Fault {
    failure
        .model_error()
        .map(runtime_model_fault)
        .unwrap_or_else(|| {
            Fault::new(
                FaultKind::TurnInternalUnexpected,
                failure.diagnostic.message().as_str(),
            )
        })
}
pub(super) fn runtime_model_fault(error: &minicore_runtime::model::ModelError) -> Fault {
    use minicore_runtime::model::ModelErrorKind;
    let kind = match error.kind() {
        ModelErrorKind::RateLimited => FaultKind::TurnModelUpstreamRateLimited,
        ModelErrorKind::ProviderUnavailable => FaultKind::TurnModelUpstreamOverloaded,
        ModelErrorKind::QuotaExceeded => FaultKind::TurnModelUpstreamQuotaExhausted,
        ModelErrorKind::AuthMissing | ModelErrorKind::AuthRejected => {
            FaultKind::TurnModelUpstreamAuthenticationFailed
        }
        ModelErrorKind::IncompleteResponse | ModelErrorKind::StreamInterrupted => {
            FaultKind::TurnModelStreamInterrupted
        }
        _ => FaultKind::TurnModelUpstreamUnclassified,
    };
    let mut fault = Fault::new(kind, error.diagnostic().message().as_str());
    if kind.allows_retry_after_ms()
        && let minicore_runtime::model::RetryHint::Retryable {
            retry_after: Some(duration),
        } = error.retry_hint()
        && let Ok(milliseconds) = u64::try_from(duration.as_millis())
    {
        fault = fault
            .with_retry_after_ms(milliseconds)
            .expect("registered temporary model fault");
    }
    fault
}
