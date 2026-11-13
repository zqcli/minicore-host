//! One Session owner for utility work, its commit claim and its original save.
//! Runtime prompt waiters may disappear; the registered task and save may not.
use super::*;
use crate::CompactionResourceFuture;
mod prompt;
use minicore_host_compaction::{self as engine, CompactionCandidate, HistorySnapshot};
use minicore_host_runtime::model::{Model, ModelCallContext};
use minicore_host_runtime::prompt::PromptError;
use minicore_host_storage::CompactionProjection;
use prompt::RequestHistory;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CompactionBinding {
    Manual {
        operation_id: OperationId,
        receipt_index: usize,
        config_revision: u64,
    },
    Automatic {
        turn_id: TurnId,
        load_id: LoadId,
        loop_id: LoopId,
        request_index: u32,
    },
}
impl CompactionBinding {
    pub(super) fn turn_id(&self) -> Option<&TurnId> {
        match self {
            Self::Automatic { turn_id, .. } => Some(turn_id),
            _ => None,
        }
    }
}

pub(super) struct CompactionWork {
    pub(super) binding: CompactionBinding,
    pub(super) activity: CompactionActivity,
    source: HistorySnapshot,
    previous_coverage: u64,
    cancellation: CancellationToken,
    candidate: Option<CompactionProjection>,
    accepted_record: Option<HistoryRecordData>,
    completion: watch::Sender<Option<Result<(), Fault>>>,
    started: Instant,
    #[cfg(test)]
    pub(super) history_waiting: Arc<tokio::sync::Semaphore>,
}
impl CompactionWork {
    fn new(
        binding: CompactionBinding,
        association: CompactionSource,
        source: HistorySnapshot,
        previous_coverage: u64,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            activity: CompactionActivity {
                source: association,
                state: CompactionState::Preparing {},
                cancellation_requested: false,
                utility_usage: Vec::new(),
            },
            binding,
            source,
            previous_coverage,
            cancellation,
            candidate: None,
            accepted_record: None,
            completion: watch::channel(None).0,
            started: Instant::now(),
            #[cfg(test)]
            history_waiting: Arc::new(tokio::sync::Semaphore::new(0)),
        }
    }
    fn committing(&self) -> bool {
        matches!(self.activity.state, CompactionState::Committing { .. })
    }
}

/// The original manual invocation's final facts, retained with its control
/// receipt. Progress never rewrites that receipt's formation-time facts.
pub(super) struct FinishedManualCompaction {
    outcome: CompactionOutcome,
    cancellation_requested: bool,
}

impl SessionController {
    pub(crate) fn compact(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: CompactRequest,
    ) -> Result<CompactReceipt, Fault> {
        self.ensure_executor()?;
        let info = self.info(caller)?;
        let preparation = self
            .resources_owned()?
            .resolve_for_compaction(info.config.clone());
        let snapshot = {
            let state = self.state.lock().expect("Session mutex poisoned");
            RequestHistory::new(&state)?
        };
        let source = snapshot.snapshot();
        let (receipt, record) = self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            self.access.ensure_execution_owned()?;
            if state.host_draining || !state.agent_enabled.load(Ordering::SeqCst) {
                return Err(Fault::new(
                    FaultKind::AgentLifecycleUnavailable,
                    "Session is not accepting utility work",
                ));
            }
            if state.active.is_some()
                || state.preparing
                || state.compaction.is_some()
                || state.history_work.is_some()
            {
                return Err(Fault::new(
                    FaultKind::SessionBusy,
                    "Session owns execution, preparation or compaction",
                ));
            }
            if !state.history_loaded {
                return Err(Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "load the Session history before compacting",
                ));
            }
            if let Some(fault) = &state.block_reason {
                return Err(fault.clone());
            }
            if state.info.config_revision != info.config_revision {
                return Err(Fault::new(
                    FaultKind::SessionConfigConflict,
                    "Session configuration changed before compaction admission",
                ));
            }
            if !snapshot.matches(&state) {
                return Err(stale());
            }
            let receipt_index = state
                .controls
                .get(&request.operation_id)
                .map_or(0, Vec::len);
            let binding = CompactionBinding::Manual {
                operation_id: request.operation_id.clone(),
                receipt_index,
                config_revision: info.config_revision,
            };
            let work = CompactionWork::new(
                binding.clone(),
                CompactionSource::Manual {
                    operation_id: request.operation_id.clone(),
                },
                source.clone(),
                snapshot.previous_coverage,
                CancellationToken::new(),
            );
            state.compaction = Some(work);
            self.publish(&mut state);
            let receipt = self.compact_receipt(&state, request.operation_id);
            Self::retain_control(&mut state, Receipt::Compact(receipt.clone()));
            let record = diagnostics::compaction_started(
                &self.identity(),
                &binding,
                &state
                    .compaction
                    .as_ref()
                    .expect("accepted compaction")
                    .activity,
            )
            .published(state.controller_revision, state.runner_revision);
            let previous = state.compaction_task.take();
            let owner = Arc::clone(self);
            state.compaction_task = Some(state.executor.as_ref().expect("checked executor").spawn(
                async move {
                    if let Some(previous) = previous {
                        let _ = previous.await;
                    }
                    let result = AssertUnwindSafe(owner.run_manual(
                        &binding,
                        &info.config,
                        snapshot,
                        preparation,
                    ))
                    .catch_unwind()
                    .await;
                    if result.is_err() {
                        owner.fail_compaction(&binding, failure("compaction worker panicked"));
                    }
                    owner.handoff_delegation_owned();
                    owner.wake();
                },
            ));
            Ok((receipt, record))
        })?;
        record.emit();
        Ok(receipt)
    }
    fn compact_receipt(&self, state: &SessionState, operation_id: OperationId) -> CompactReceipt {
        let work = state.compaction.as_ref().expect("accepted compaction");
        CompactReceipt {
            kind: CompactKind::Compact,
            host_id: self.host_id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id,
            agent_id: self.agent_id.clone(),
            session_id: self.id.clone(),
            result: Self::compact_facts(state, &work.activity),
        }
    }
    fn compact_facts(state: &SessionState, activity: &CompactionActivity) -> CompactFacts {
        CompactFacts {
            state: activity.state.clone(),
            cancellation_requested: activity.cancellation_requested,
            controller_revision: state.controller_revision,
        }
    }
    fn publish_compaction(&self, state: &mut SessionState) {
        self.publish(state);
    }
    pub(crate) fn compact_cancel(
        &self,
        caller: &CallerContext,
        request: CompactCancelRequest,
    ) -> Result<CompactCancelReceipt, Fault> {
        let (receipt, record) = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::available(&state)?;
            let targets = state
                .controls
                .get(&request.target_operation_id)
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(index, receipt)| {
                    matches!(receipt, Receipt::Compact(_)).then_some(index)
                })
                .collect::<Vec<_>>();
            let receipt_index = match targets.as_slice() {
                [] => {
                    return Err(Fault::new(
                        FaultKind::SessionCompactionNotFound,
                        "the original manual compaction is not retained by this Session",
                    ));
                }
                [index] => *index,
                _ => {
                    return Err(Fault::new(
                        FaultKind::HostReceiptAmbiguous,
                        "operation_id identifies multiple compaction invocations",
                    ));
                }
            };
            let current = state.compaction.as_ref().is_some_and(|work| {
                matches!(&work.binding,
                CompactionBinding::Manual { operation_id, receipt_index: index, .. }
                    if operation_id == &request.target_operation_id && *index == receipt_index)
            });
            let claimed = current && Self::cancel_compaction_locked(&mut state);
            self.publish_compaction(&mut state);
            let record = claimed.then(|| {
                let work = state
                    .compaction
                    .as_ref()
                    .expect("claimed original cancellation");
                diagnostics::compaction_cancelled(
                    &self.identity(),
                    &work.binding,
                    &work.activity,
                    Some(&request.operation_id),
                )
                .published(state.controller_revision, state.runner_revision)
            });
            let target = if current {
                Self::compact_facts(
                    &state,
                    &state.compaction.as_ref().expect("current work").activity,
                )
            } else {
                let finished = state
                    .finished_manual_compactions
                    .get(&(request.target_operation_id.clone(), receipt_index))
                    .expect("retained manual receipt has its original completion facts");
                CompactFacts {
                    state: CompactionState::Finished {
                        result: finished.outcome.clone(),
                    },
                    cancellation_requested: finished.cancellation_requested,
                    controller_revision: state.controller_revision,
                }
            };
            let receipt = CompactCancelReceipt {
                kind: CompactCancelKind::CompactCancel,
                host_id: self.host_id.clone(),
                host_incarnation: self.incarnation.clone(),
                operation_id: request.operation_id,
                target_operation_id: request.target_operation_id,
                agent_id: self.agent_id.clone(),
                session_id: self.id.clone(),
                result: CompactCancelResult {
                    cancel_claimed: claimed,
                    target,
                },
            };
            Self::retain_control(&mut state, Receipt::CompactCancel(receipt.clone()));
            Ok((receipt, record))
        })?;
        diagnostic::emit(record);
        Ok(receipt)
    }
    pub(super) fn cancel_compaction_locked(state: &mut SessionState) -> bool {
        let Some(work) = &mut state.compaction else {
            return false;
        };
        if work.committing() || work.activity.cancellation_requested {
            return false;
        }
        work.activity.cancellation_requested = true;
        work.cancellation.cancel();
        true
    }
    async fn run_manual(
        self: &Arc<Self>,
        binding: &CompactionBinding,
        config: &SessionConfig,
        history: RequestHistory,
        preparation: CompactionResourceFuture,
    ) {
        let cancellation = {
            let state = self.state.lock().expect("Session mutex poisoned");
            state
                .compaction
                .as_ref()
                .expect("registered manual work")
                .cancellation
                .clone()
        };
        let source = {
            let state = self.state.lock().expect("Session mutex poisoned");
            state
                .compaction
                .as_ref()
                .expect("registered manual work")
                .source
                .clone()
        };
        if source.items.is_empty() {
            self.complete_compaction(binding, CompactionOutcome::Unchanged, Ok(()));
            return;
        }
        let prepared = tokio::select! {
            _ = cancellation.cancelled() => Err(PromptError::Cancelled),
            result = tokio::time::timeout(Duration::from_millis(config.execution.prompt_timeout_ms), preparation) => match result {
                Ok(Ok(resources)) => Ok(resources),
                Ok(Err(fault)) => { self.fail_compaction(binding, fault); return; }
                Err(_) => Err(PromptError::Cancelled),
            },
        };
        let resources = match prepared {
            Ok(value) => value,
            Err(_) => {
                self.fail_compaction(
                    binding,
                    failure("compaction preparation cancelled or timed out"),
                );
                return;
            }
        };
        let loop_id = match LoopId::new() {
            Ok(id) => id,
            Err(error) => {
                self.fail_compaction(binding, failure(error.to_string()));
                return;
            }
        };
        let prompt_deadline =
            tokio::time::Instant::now() + Duration::from_millis(config.execution.prompt_timeout_ms);
        let plan = history
            .prepare_manual(
                source,
                &resources,
                loop_id,
                &config.execution.compaction,
                &cancellation,
                prompt_deadline,
            )
            .await;
        match plan {
            Ok(Some(plan)) if self.plan_advances(binding, &plan) => {
                let deadline = tokio::time::Instant::now()
                    + Duration::from_millis(config.execution.model_timeout_ms);
                self.generate_and_commit(
                    binding,
                    plan,
                    resources.model,
                    ModelCallContext::new(loop_id, 0, cancellation, deadline.into_std()),
                )
                .await
            }
            Ok(_) => self.complete_compaction(binding, CompactionOutcome::Unchanged, Ok(())),
            Err(fault) => self.fail_compaction(binding, fault),
        }
    }
    fn plan_advances(&self, binding: &CompactionBinding, plan: &engine::CompactionPlan) -> bool {
        let state = self.state.lock().expect("Session mutex poisoned");
        state
            .compaction
            .as_ref()
            .filter(|work| &work.binding == binding)
            .is_some_and(|work| plan.covered().end > usize::from(work.previous_coverage > 0))
    }
    async fn generate_and_commit(
        self: &Arc<Self>,
        binding: &CompactionBinding,
        plan: engine::CompactionPlan,
        model: Arc<dyn Model>,
        context: ModelCallContext,
    ) {
        {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let Some(work) = state
                .compaction
                .as_mut()
                .filter(|work| &work.binding == binding)
            else {
                return;
            };
            if !self.access.execution_allowed_owned() {
                work.activity.cancellation_requested = true;
                work.cancellation.cancel();
            }
            work.activity.state = CompactionState::Summarizing {};
            self.publish_compaction(&mut state);
        }
        let utility_loop = context.loop_id;
        let candidate = match engine::generate(&plan, model.as_ref(), context).await {
            Ok(candidate) => candidate,
            Err(error) => {
                self.record_utility(binding, utility_loop, &error.utility_usage);
                self.fail_compaction(binding, failure(error.error.to_string()));
                return;
            }
        };
        self.record_utility(binding, utility_loop, &candidate.utility_usage);
        let save = match self.claim_compaction_commit(binding, &candidate) {
            Ok(save) => save,
            Err(fault) => {
                self.fail_compaction(binding, fault);
                return;
            }
        };
        let result = history::checked_save(async { self.backend.append(save.1).await }).await;
        let _ = self.settle_save(&save.0, result);
        self.checkpoint_activity();
    }
    fn record_utility(
        &self,
        binding: &CompactionBinding,
        loop_id: LoopId,
        usage: &[engine::UtilityUsage],
    ) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let Some(work) = state
            .compaction
            .as_mut()
            .filter(|work| &work.binding == binding)
        else {
            return;
        };
        work.activity.utility_usage = usage
            .iter()
            .map(|value| CompactionUsage {
                request_index: value.request_index,
                usage: value.usage,
                completed: value.completed,
            })
            .collect();
        work.activity.state = CompactionState::Merging {};
        let records =
            diagnostics::compaction_utilities(&self.identity(), binding, &work.activity, loop_id);
        self.publish_compaction(&mut state);
        let records = records
            .into_iter()
            .map(|record| record.published(state.controller_revision, state.runner_revision))
            .collect::<Vec<_>>();
        drop(state);
        diagnostic::emit(records);
    }
    fn claim_compaction_commit(
        self: &Arc<Self>,
        binding: &CompactionBinding,
        candidate: &CompactionCandidate,
    ) -> Result<(history::SaveBinding, Vec<HistoryRecord>), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let work = state
            .compaction
            .as_ref()
            .filter(|work| &work.binding == binding)
            .ok_or_else(stale)?;
        if work.cancellation.is_cancelled()
            || state.host_stopping
            || !self.access.execution_allowed_owned()
            || state.lifecycle != SessionLifecycle::Open
        {
            return Err(failure("compaction cancelled before commit"));
        }
        match binding {
            CompactionBinding::Manual {
                config_revision, ..
            } if state.active.is_none()
                && !state.preparing
                && state.info.config_revision == *config_revision => {}
            CompactionBinding::Automatic {
                turn_id,
                load_id,
                loop_id,
                ..
            } if state.active.as_ref().is_some_and(|active| {
                &active.info.id == turn_id
                    && &active.info.load_id == load_id
                    && active.loop_id == *loop_id
                    && active.info.state == TurnState::Running
            }) => {}
            _ => return Err(stale()),
        }
        if state.history_work.is_some()
            || state.history_position != candidate.source().revision
            || !candidate.matches(&work.source)
            || state.projection.as_ref().map_or(0, |p| p.covered_items) != work.previous_coverage
        {
            return Err(stale());
        }
        let covered = work.previous_coverage + candidate.covered().end as u64
            - u64::from(work.previous_coverage > 0);
        // Inputs already accepted for this Turn have published native positions.
        // Automatic compaction must append that original unsaved prefix before
        // its marker, within this same registered save; it may not overtake them.
        let mut records = if let Some(active) = &state.active {
            state.history[active.save_cursor..]
                .iter()
                .enumerate()
                .map(|(offset, item)| {
                    let position = state.history_position + offset as u64;
                    let entry = state.input_provenance.entry(
                        active.save_cursor + offset,
                        &active.info.id,
                        item,
                    )?;
                    let recorded_at = state.readable_history.as_ref().expect("loaded history")
                        [position as usize]
                        .recorded_at;
                    Ok(HistoryRecord::at(
                        self.id.clone(),
                        position,
                        recorded_at,
                        entry,
                    ))
                })
                .collect::<Result<Vec<_>, Fault>>()?
        } else {
            Vec::new()
        };
        let source_position = state.history_position + records.len() as u64;
        let projection =
            CompactionProjection::new(source_position, covered, candidate.summary.clone());
        projection
            .validate(&state.history, source_position, work.previous_coverage)
            .map_err(|error| failure(error.to_string()))?;
        let change = CompactionChange {
            source_position,
            covered_items: covered,
            before_tokens: candidate.before_tokens,
            after_tokens: candidate.after_tokens,
        };
        let recorded_at = Timestamp::now();
        let entry = HistoryRecord::at(
            self.id.clone(),
            source_position,
            recorded_at,
            projection.clone().into_entry(),
        );
        state.info.last_activity_at = state.info.last_activity_at.max(recorded_at);
        let work = state.compaction.as_mut().expect("checked work");
        work.candidate = Some(projection);
        work.accepted_record = Some(HistoryRecordData::from_record(&entry));
        work.activity.state = CompactionState::Committing {
            change,
            persistence: PersistenceState::Pending,
            fault: None,
        };
        records.push(entry);
        let saved_native_end = state.history.len();
        if let Some(active) = state.active.as_mut() {
            active.save_cursor = saved_native_end;
        }
        let save = self.register_compaction_save(&mut state, binding.clone(), records.len());
        self.checkpoint_activity_locked(&mut state);
        self.publish_compaction(&mut state);
        let record = diagnostics::compaction_committing(
            &self.identity(),
            binding,
            &state
                .compaction
                .as_ref()
                .expect("original candidate")
                .activity,
        )
        .published(state.controller_revision, state.runner_revision);
        drop(state);
        record.emit();
        Ok((save, records))
    }
    pub(super) fn settle_compaction_save(
        &self,
        state: &mut SessionState,
        binding: &CompactionBinding,
        result: &Result<PersistenceState, Fault>,
    ) -> Result<Vec<Record>, Fault> {
        let work = state
            .compaction
            .as_mut()
            .filter(|work| &work.binding == binding)
            .ok_or_else(stale)?;
        let CompactionState::Committing {
            change,
            persistence,
            fault,
        } = &mut work.activity.state
        else {
            return Err(stale());
        };
        let records = match result {
            Ok(confirmed) => {
                let result = CompactionOutcome::Compacted {
                    change: change.clone(),
                    persistence: *confirmed,
                };
                state.projection = work.candidate.take();
                state
                    .readable_history
                    .as_mut()
                    .expect("loaded history")
                    .push(
                        work.accepted_record
                            .take()
                            .expect("original accepted compaction record"),
                    );
                self.complete_compaction_locked(state, binding, result, Ok(()))
            }
            Err(error) => {
                *persistence = PersistenceState::Failed;
                *fault = Some(error.clone());
                let _ = work.completion.send_replace(Some(Err(error.clone())));
                Self::claim_compaction_failure(state, binding, error, true);
                self.publish_compaction(state);
                Vec::new()
            }
        };
        Ok(records)
    }
    fn fail_compaction(&self, binding: &CompactionBinding, fault: Fault) {
        self.complete_compaction(
            binding,
            CompactionOutcome::Failed {
                fault: fault.clone(),
            },
            Err(fault),
        );
    }
    fn complete_compaction(
        &self,
        binding: &CompactionBinding,
        result: CompactionOutcome,
        completion: Result<(), Fault>,
    ) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let records = self.complete_compaction_locked(&mut state, binding, result, completion);
        drop(state);
        diagnostic::emit(records);
    }
    fn complete_compaction_locked(
        &self,
        state: &mut SessionState,
        binding: &CompactionBinding,
        mut result: CompactionOutcome,
        mut completion: Result<(), Fault>,
    ) -> Vec<Record> {
        let Some(work) = state
            .compaction
            .as_ref()
            .filter(|work| &work.binding == binding)
        else {
            return Vec::new();
        };
        if work.committing() && !matches!(result, CompactionOutcome::Compacted { .. }) {
            return Vec::new();
        }
        if !work.committing() && work.cancellation.is_cancelled() {
            result = CompactionOutcome::Cancelled;
            completion = Err(failure("compaction cancelled"));
        }
        let mut work = state.compaction.take().expect("checked work");
        work.activity.cancellation_requested |=
            work.cancellation.is_cancelled() && !work.committing();
        if let CompactionOutcome::Failed { fault } = &result {
            Self::claim_compaction_failure(state, binding, fault, false);
        }
        if let CompactionBinding::Manual {
            operation_id,
            receipt_index,
            ..
        } = binding
        {
            state.finished_manual_compactions.insert(
                (operation_id.clone(), *receipt_index),
                FinishedManualCompaction {
                    outcome: result.clone(),
                    cancellation_requested: work.activity.cancellation_requested,
                },
            );
        }
        work.activity.state = CompactionState::Finished { result };
        let record = diagnostics::compaction_finished(
            &self.identity(),
            binding,
            &work.activity,
            work.started,
        );
        work.completion.send_replace(Some(completion));
        state.last_compaction = Some(work.activity.clone());
        let previous = state.lifecycle;
        Self::finish_memory_close(state);
        self.publish(state);
        let mut records = vec![record.published(state.controller_revision, state.runner_revision)];
        records.extend(self.closed_record(state, previous));
        records
    }
    pub(super) fn claim_compaction_failure(
        state: &mut SessionState,
        binding: &CompactionBinding,
        fault: &Fault,
        history: bool,
    ) {
        let CompactionBinding::Automatic {
            turn_id,
            load_id,
            loop_id,
            request_index,
        } = binding
        else {
            return;
        };
        let Some(active) = state.active.as_mut().filter(|active| {
            &active.info.id == turn_id
                && &active.info.load_id == load_id
                && active.loop_id == *loop_id
                && active.info.requests == u64::from(*request_index)
        }) else {
            return;
        };
        if active.info.outcome.is_some() {
            if !history {
                active.info.cleanup_faults.push(fault.clone());
            }
        } else {
            active.claim_terminal(TurnOutcome::Failed, (!history).then(|| fault.clone()));
        }
    }
    /// A cancelled preparation waiter does not abandon its registered save.
    /// Fence the original compaction before appending subsequent records.
    pub(super) async fn wait_compaction_boundary(&self) -> Result<(), Fault> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.state.lock().expect("Session mutex poisoned");
                if state.compaction.is_none() {
                    return Ok(());
                }
                #[cfg(test)]
                if let Some(work) = &state.compaction {
                    work.history_waiting.add_permits(1);
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
    #[allow(clippy::too_many_arguments)]
    fn start_automatic(
        self: &Arc<Self>,
        binding: CompactionBinding,
        source: HistorySnapshot,
        previous_coverage: u64,
        plan: engine::CompactionPlan,
        model: Arc<dyn Model>,
        cancellation: CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<watch::Receiver<Option<Result<(), Fault>>>, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.host_stopping
            || !self.access.execution_allowed_owned()
            || state.lifecycle != SessionLifecycle::Open
            || state.compaction.is_some()
            || state.history_work.is_some()
            || state.history_position != source.revision
            || cancellation.is_cancelled()
        {
            return Err(stale());
        }
        let CompactionBinding::Automatic {
            turn_id,
            load_id,
            loop_id,
            request_index,
        } = &binding
        else {
            return Err(stale());
        };
        let _active = state
            .active
            .as_ref()
            .filter(|active| {
                &active.info.id == turn_id
                    && &active.info.load_id == load_id
                    && active.loop_id == *loop_id
                    && active.info.state == TurnState::Running
            })
            .ok_or_else(stale)?;
        // Map at the same owner boundary that validates the original internal
        // Loop. A continuation's request zero is not public response zero.
        let association = CompactionSource::Automatic {
            turn_id: turn_id.clone(),
            load_id: load_id.clone(),
            response_index: u64::from(*request_index),
        };
        let context = ModelCallContext::new(
            *loop_id,
            *request_index,
            cancellation.clone(),
            deadline.into_std(),
        );
        let work = CompactionWork::new(
            binding.clone(),
            association,
            source,
            previous_coverage,
            cancellation,
        );
        let completion = work.completion.subscribe();
        state.compaction = Some(work);
        self.publish_compaction(&mut state);
        let record = diagnostics::compaction_started(
            &self.identity(),
            &binding,
            &state
                .compaction
                .as_ref()
                .expect("accepted compaction")
                .activity,
        )
        .published(state.controller_revision, state.runner_revision);
        let previous = state.compaction_task.take();
        let owner = Arc::clone(self);
        state.compaction_task = Some(
            state
                .executor
                .as_ref()
                .expect("running Turn executor")
                .spawn(async move {
                    if let Some(previous) = previous {
                        let _ = previous.await;
                    }
                    if AssertUnwindSafe(owner.generate_and_commit(&binding, plan, model, context))
                        .catch_unwind()
                        .await
                        .is_err()
                    {
                        owner.fail_compaction(
                            &binding,
                            failure("automatic compaction worker panicked"),
                        );
                    }
                    owner.handoff_delegation_owned();
                }),
        );
        drop(state);
        record.emit();
        Ok(completion)
    }
}
fn failure(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::SessionCompactionFailed, message)
}
fn stale() -> Fault {
    failure("compaction no longer matches its original Session history and execution")
}
