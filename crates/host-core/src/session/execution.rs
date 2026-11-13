//! Public Turn scheduling and Runtime orchestration owned by this Session.
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
                resources.execution.descriptor(),
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
            let loop_id = match LoopId::new() {
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
                    loop_id,
                    cancellation: CancellationToken::new(),
                    consumed_inputs: Vec::new(),
                    bound_inputs: batch
                        .iter()
                        .map(|input| input.message.message_id.clone())
                        .collect(),
                    start_history,
                    save_cursor: start_history,
                    partials: Vec::new(),
                    pending_partial: None,
                    deadline_expired: false,
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
        batch: Vec<QueuedInput>,
        resources: BoundResources,
    ) {
        {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            self.accept_bound_inputs(&mut state, batch);
            let active = state.active.as_mut().expect("bound Turn");
            if active.info.state == TurnState::Starting {
                active.info.state = TurnState::Running;
            }
            self.publish(&mut state);
        }
        let result = AssertUnwindSafe(self.run_steps(&turn_id, resources))
            .catch_unwind()
            .await;
        let (outcome, fault) = result.unwrap_or_else(|_| {
            (
                TurnOutcome::Failed,
                Some(Fault::new(
                    FaultKind::TurnInternalUnexpected,
                    "Turn execution panicked",
                )),
            )
        });
        self.finish_turn(&turn_id, outcome, fault).await;
    }

    async fn run_steps(
        self: &Arc<Self>,
        turn_id: &TurnId,
        resources: BoundResources,
    ) -> (TurnOutcome, Option<Fault>) {
        use minicore_host_runtime::model::{
            AssistantPart, ModelCallParams, ModelDriver, ModelDriverConfig, ModelFinishReason,
        };
        use minicore_host_runtime::tools::{ToolBatchContext, ToolBatchStop, run_tool_batch};
        let execution = match self.resources_owned().map(|resolver| {
            let execution = resolver.bind_tool_output(
                &resources.execution,
                ToolOutputBinding::new(self, turn_id.clone()),
            );
            self.bind_turn_model(turn_id, &execution)
        }) {
            Ok(execution) => execution,
            Err(fault) => return (TurnOutcome::Failed, Some(fault)),
        };
        if let Err(error) = resources.options.validate() {
            return (
                TurnOutcome::Failed,
                Some(Fault::new(
                    FaultKind::TurnInternalUnexpected,
                    error.to_string(),
                )),
            );
        }
        let options = &resources.options;
        let driver = match ModelDriver::new(
            execution.model().as_ref(),
            ModelDriverConfig::new(
                options.model_timeout,
                options.model_retry_attempts,
                options.model_retry_base_delay,
            ),
        ) {
            Ok(driver) => driver,
            Err(error) => return (TurnOutcome::Failed, Some(runtime_model_fault(&error))),
        };
        let (loop_id, cancellation) = {
            let state = self.state.lock().expect("Session mutex poisoned");
            let active = state.active.as_ref().expect("owned Turn");
            (active.loop_id, active.cancellation.clone())
        };
        loop {
            // Rebuilding this same request for newly accepted steering does
            // not reset its preparation deadline or its response index.
            let mut prompt_deadline = options
                .deadline
                .unwrap_or_else(|| tokio::time::Instant::now() + options.prompt_timeout)
                .min(tokio::time::Instant::now() + options.prompt_timeout);
            let request_index = {
                let state = self.state.lock().expect("Session mutex poisoned");
                match u32::try_from(state.active.as_ref().expect("owned Turn").info.requests) {
                    Ok(index) => index,
                    Err(_) => {
                        return (
                            TurnOutcome::Failed,
                            Some(Fault::new(
                                FaultKind::TurnInternalUnexpected,
                                "response index exhausted",
                            )),
                        );
                    }
                }
            };
            let request = loop {
                if self.turn_stopped(turn_id, options.deadline) {
                    return (TurnOutcome::Cancelled, None);
                }
                self.consume_steering(turn_id);
                let prepared = self
                    .prepare_request(
                        turn_id,
                        request_index,
                        &execution,
                        &cancellation,
                        &mut prompt_deadline,
                        options,
                    )
                    .await;
                if self.turn_stopped(turn_id, options.deadline) {
                    return (TurnOutcome::Cancelled, None);
                }
                let request = match prepared {
                    Ok(request) => request,
                    Err(fault) => {
                        return (
                            TurnOutcome::Failed,
                            (fault.kind() != Some(FaultKind::SessionHistoryWriteFailed))
                                .then_some(fault),
                        );
                    }
                };
                if self.consume_steering(turn_id) {
                    continue;
                }
                // Admission after all asynchronous preparation shares the same
                // lock as cancellation and input claims. Later input belongs
                // to the next permitted boundary.
                if !self.begin_model_request(turn_id, request_index, options.deadline) {
                    if self.turn_stopped(turn_id, options.deadline) {
                        return (TurnOutcome::Cancelled, None);
                    }
                    continue;
                }
                break request;
            };
            let params = ModelCallParams {
                loop_id,
                request_index,
                cancellation: cancellation.clone(),
                deadline: options.deadline.map(tokio::time::Instant::into_std),
            };
            let report = driver
                .call(request, params, &mut |event| {
                    self.observe_model_call(turn_id, request_index, event)
                })
                .await;
            let source = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let active = state.active.as_mut().expect("owned Turn");
                active
                    .info
                    .model_attempts
                    .resize(request_index as usize + 1, 0);
                active.info.model_attempts[request_index as usize] = report.attempts;
                active.info.model_retry = None;
                let source = self.turn_identity(&active.info);
                self.publish(&mut state);
                source
            };
            diagnostic::emit(diagnostics::model_reported(
                &source,
                loop_id,
                request_index,
                &report,
            ));
            let response = match report.outcome {
                Ok(response) => response,
                Err(mut failure) => {
                    let deadline = failure.deadline_source()
                        == Some(minicore_host_runtime::DeadlineSource::Turn);
                    let stopped = self.turn_stopped(turn_id, options.deadline) || deadline;
                    {
                        let mut state = self.state.lock().expect("Session mutex poisoned");
                        let active = state.active.as_mut().expect("owned Turn");
                        active.pending_partial = failure.partial.take().map(|value| *value);
                        active.deadline_expired |= deadline;
                    }
                    return if stopped {
                        (TurnOutcome::Cancelled, None)
                    } else {
                        (
                            TurnOutcome::Failed,
                            Some(runtime_model_fault(failure.error())),
                        )
                    };
                }
            };
            let calls = response
                .parts()
                .iter()
                .filter_map(AssistantPart::as_tool_call)
                .collect::<Vec<_>>();
            let finish = calls.is_empty().then(|| match response.finish_reason() {
                ModelFinishReason::Stop => (TurnOutcome::Completed, None),
                ModelFinishReason::Unknown | ModelFinishReason::ToolCalls => {
                    let error = minicore_host_runtime::model::ModelError::started(
                        minicore_host_runtime::model::ModelErrorKind::InvalidProviderResponse,
                        DiagnosticSummary::new(
                            DiagnosticCode::ModelMalformedResponse,
                            DiagnosticCategory::Model,
                            Text::new("model returned an invalid response"),
                            false,
                        )
                        .expect("static diagnostic"),
                    );
                    (TurnOutcome::Failed, Some(runtime_model_fault(&error)))
                }
                reason => (
                    TurnOutcome::Failed,
                    Some(Fault::new(
                        FaultKind::TurnInternalUnexpected,
                        match reason {
                            ModelFinishReason::Length => {
                                "model output exceeded the configured limit"
                            }
                            ModelFinishReason::Refused => "model refused to answer",
                            ModelFinishReason::ContentFiltered => {
                                "model response was content filtered"
                            }
                            _ => unreachable!(),
                        },
                    )),
                ),
            });
            if let Some((TurnOutcome::Failed, fault)) = &finish {
                self.claim_turn(turn_id, TurnOutcome::Failed, fault.clone());
            }
            let item = HistoryItem::Assistant(minicore_host_runtime::AssistantHistory {
                loop_id,
                request_index,
                model: execution.descriptor().model_ref.clone(),
                reasoning: execution.reasoning(),
                content: response.parts().to_vec(),
                provider_replay: response.provider_replay().cloned(),
                finish_reason: response.finish_reason(),
                usage: response.usage().copied().unwrap_or_default(),
            });
            if self.save_history(turn_id, &[item]).await.is_err() {
                return finish.unwrap_or((TurnOutcome::Failed, None));
            }
            if let Some((outcome, fault)) = finish {
                if outcome == TurnOutcome::Failed {
                    return (outcome, fault);
                }
                let (continued, records) = {
                    let mut state = self.state.lock().expect("Session mutex poisoned");
                    if self.stop_turn_locked(&mut state, turn_id, options.deadline) {
                        (false, Vec::new())
                    } else if self.schedulable(&state) && !state.next_step.is_empty() {
                        let batch = take_batch(&mut state, false, turn_id);
                        let records = self.observe_bound(&state, &batch);
                        self.accept_bound_inputs(&mut state, batch);
                        self.publish(&mut state);
                        (true, records)
                    } else {
                        state
                            .active
                            .as_mut()
                            .expect("owned Turn")
                            .claim_terminal(TurnOutcome::Completed, None);
                        self.publish(&mut state);
                        (false, Vec::new())
                    }
                };
                diagnostic::emit(records);
                if continued {
                    continue;
                }
                return (TurnOutcome::Completed, None);
            }
            if self.turn_stopped(turn_id, options.deadline) {
                // The batch still authors cancellation results for these
                // accepted calls. The primitive cannot start cancelled work.
                cancellation.cancel();
            }
            {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let active = state.active.as_mut().expect("owned Turn");
                active.info.tool_rounds += 1;
                active.info.phase = Some(TurnPhase::Tools);
                self.publish(&mut state);
            }
            let batch = run_tool_batch(
                &execution,
                &calls,
                ToolBatchContext {
                    loop_id,
                    request_index,
                    cancellation: cancellation.clone(),
                    deadline: options.deadline.map(|deadline| deadline.into_std()),
                    policy_timeout: options.policy_timeout,
                    tool_timeout: options.tool_timeout,
                },
                Arc::new(super::interaction::SessionToolHost {
                    session: Arc::clone(self),
                    turn_id: turn_id.clone(),
                }),
            )
            .await;
            let stop = batch.stop.map(|stop| match stop {
                ToolBatchStop::Cancelled => (TurnOutcome::Cancelled, None),
                ToolBatchStop::Deadline => {
                    self.state
                        .lock()
                        .expect("Session mutex poisoned")
                        .active
                        .as_mut()
                        .expect("owned Turn")
                        .deadline_expired = true;
                    (TurnOutcome::Cancelled, None)
                }
                ToolBatchStop::Policy(diagnostic)
                | ToolBatchStop::Interaction(diagnostic)
                | ToolBatchStop::Internal(diagnostic) => (
                    TurnOutcome::Failed,
                    Some(Fault::new(
                        FaultKind::TurnInternalUnexpected,
                        diagnostic.message().as_str(),
                    )),
                ),
            });
            if let Some((outcome, fault)) = &stop {
                self.claim_turn(turn_id, *outcome, fault.clone());
            }
            let results = batch
                .results
                .into_iter()
                .map(HistoryItem::ToolResult)
                .collect::<Vec<_>>();
            if self.save_history(turn_id, &results).await.is_err() {
                return stop.unwrap_or((TurnOutcome::Failed, None));
            }
            if let Some(stop) = stop {
                return stop;
            }
        }
    }

    pub(super) fn stop_turn_locked(
        &self,
        state: &mut SessionState,
        turn_id: &TurnId,
        deadline: Option<tokio::time::Instant>,
    ) -> bool {
        let force = state.host_stopping || !self.access.execution_allowed_owned();
        let Some(active) = state
            .active
            .as_mut()
            .filter(|active| &active.info.id == turn_id)
        else {
            return true;
        };
        let expired = deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now());
        if force
            || expired
            || active.cancellation.is_cancelled()
            || active.info.state == TurnState::Cancelling
        {
            if active.info.outcome.is_none() {
                active.info.state = TurnState::Cancelling;
                active._activation.expire();
            }
            active.deadline_expired |= expired;
            active.cancellation.cancel();
            true
        } else {
            active.info.outcome.is_some()
        }
    }
    fn turn_stopped(&self, turn_id: &TurnId, deadline: Option<tokio::time::Instant>) -> bool {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let stopped = self.stop_turn_locked(&mut state, turn_id, deadline);
        if stopped {
            self.publish(&mut state);
        }
        stopped
    }
    fn consume_steering(&self, turn_id: &TurnId) -> bool {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if !self.schedulable(&state)
            || state
                .active
                .as_ref()
                .is_none_or(|a| &a.info.id != turn_id || a.info.state != TurnState::Running)
            || state.next_step.is_empty()
        {
            return false;
        }
        let batch = take_batch(&mut state, false, turn_id);
        let records = self.observe_bound(&state, &batch);
        self.accept_bound_inputs(&mut state, batch);
        self.publish(&mut state);
        drop(state);
        diagnostic::emit(records);
        true
    }
    pub(super) fn claim_turn(&self, turn_id: &TurnId, outcome: TurnOutcome, fault: Option<Fault>) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let previous = diagnostics::Schedule::capture(&state);
        let Some(active) = state.active.as_mut().filter(|a| &a.info.id == turn_id) else {
            return;
        };
        let newly_claimed = active.claim_terminal(outcome, fault);
        if active.info.outcome == Some(TurnOutcome::Failed) {
            state.failure_paused = true;
        }
        self.publish(&mut state);
        let record = self.terminal_claim_record(&state, newly_claimed);
        let scheduling = self.scheduling_record(&state, previous, "finalizing", None);
        drop(state);
        diagnostic::emit(record);
        diagnostic::emit(scheduling);
    }
    pub(super) async fn finish_turn(
        &self,
        turn_id: &TurnId,
        outcome: TurnOutcome,
        fault: Option<Fault>,
    ) {
        self.claim_turn(turn_id, outcome, fault);
        let saved = self.save_history(turn_id, &[]).await.is_ok()
            && self.save_partial(turn_id).await.is_ok();
        // Saving never withdraws the original worker cleanup responsibility.
        let cleaned = self.finish_tool_cleanup(turn_id).await;
        if !saved || !cleaned {
            return;
        }
        if !self.save_terminal(turn_id).await {
            return;
        }
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
        self.observe_finished(&mut state, &result.info, active.deadline_expired);
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

pub(super) fn runtime_model_fault(error: &minicore_host_runtime::model::ModelError) -> Fault {
    use minicore_host_runtime::model::ModelErrorKind;
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
        && let minicore_host_runtime::model::RetryHint::Retryable {
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
