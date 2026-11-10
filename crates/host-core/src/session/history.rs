use super::*;

/// These are existing execution identities and positions, not a public record
/// identifier or a replay queue. At most one accepted save may be unconfirmed.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Boundary {
    Complete { loop_id: LoopId, end: usize },
    Partial { loop_id: LoopId },
    Terminal,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Binding {
    turn_id: TurnId,
    load_id: LoadId,
    boundary: Boundary,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SaveBinding {
    Turn(Binding),
    Compaction(compaction::CompactionBinding),
}
pub(super) struct HistorySaveWork {
    binding: SaveBinding,
    running: bool,
    pub(super) fault: Option<Fault>,
}
impl SessionController {
    pub(super) async fn reconcile_report(
        self: &Arc<Self>,
        turn_id: &TurnId,
        report: &LoopReport,
    ) -> Result<(), Fault> {
        let accepted = {
            let state = self.state.lock().expect("Session mutex poisoned");
            if let Some(work) = &state.history_work {
                return Err(work.fault.clone().unwrap_or_else(|| {
                    Fault::new(
                        FaultKind::SessionHistoryWriteFailed,
                        "original save is pending",
                    )
                }));
            }
            let active = state.active.as_ref().ok_or_else(stale_history)?;
            if &active.info.id != turn_id
                || active.loop_id != Some(report.loop_id)
                || active.accepted > report.appended.len()
            {
                return Err(stale_history());
            }
            active.accepted
        };
        // history_failure records the original callback failure. It is not a
        // second business result and cannot prevent an explicit original-range
        // confirmation from settling the report already returned by that Loop.
        if accepted < report.appended.len() {
            self.accept_history(
                turn_id,
                report.loop_id,
                accepted,
                &report.appended[accepted..],
            )
            .await?;
        }
        let (binding, entries) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let active = state.active.as_ref().ok_or_else(stale_history)?;
            if &active.info.id != turn_id || active.loop_id != Some(report.loop_id) {
                return Err(stale_history());
            }
            let Some(partial) = report.partial.as_ref().filter(|_| !active.partial_accepted) else {
                return Ok(());
            };
            let response_index = active.response_base + u64::from(partial.request_index);
            let encoded_index = u32::try_from(response_index).map_err(|_| stale_history())?;
            let entries = vec![HistoryEntry::Partial {
                turn_id: turn_id.clone(),
                loop_id: report.loop_id,
                response_index: encoded_index,
                partial: partial.clone(),
            }];
            let binding = Self::register_turn_save(
                &mut state,
                turn_id,
                Boundary::Partial {
                    loop_id: report.loop_id,
                },
                entries.len(),
            )?;
            let active = state.active.as_mut().expect("checked original Turn");
            active.partials.push(PartialResult {
                response_index,
                partial: partial.clone(),
            });
            active.partial_accepted = true;
            state
                .readable_history
                .as_mut()
                .expect("history loaded before execution")
                .extend(entries.iter().filter_map(HistoryItemData::from_entry));
            self.publish(&mut state);
            (binding, entries)
        };
        self.persist_registered(binding, entries).await
    }
    pub(super) async fn accept_history(
        &self,
        turn_id: &TurnId,
        loop_id: LoopId,
        offset: usize,
        items: &[HistoryItem],
    ) -> Result<(), Fault> {
        self.wait_compaction_boundary().await?;
        let (binding, entries, diagnostics) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let active = state.active.as_ref().ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "history callback has no active Turn",
                )
            })?;
            if &active.info.id != turn_id
                || active.loop_id != Some(loop_id)
                || active.accepted != offset
            {
                return Err(Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "history callback does not match its original execution range",
                ));
            }
            let mut entries = Vec::new();
            let mut consumed = Vec::new();
            let mut pending = active.bound_pending.iter();
            for item in items {
                match item {
                    HistoryItem::User(user) => {
                        let input = pending.next().ok_or_else(|| {
                            Fault::new(
                                FaultKind::SessionInternalUnexpected,
                                "Runtime returned an unbound input",
                            )
                        })?;
                        if input.runtime != user.input {
                            return Err(Fault::new(
                                FaultKind::SessionInternalUnexpected,
                                "Runtime input order differs from bound messages",
                            ));
                        }
                        consumed.push(input.message.message_id.clone());
                        entries.push(HistoryEntry::Input {
                            turn_id: turn_id.clone(),
                            message_id: input.message.message_id.clone(),
                            source: input.source.clone(),
                            mode: input.message.mode,
                            in_reply_to: input.message.in_reply_to.clone(),
                            user: user.clone(),
                        });
                    }
                    _ => {
                        let request_index = match item {
                            HistoryItem::Assistant(value) => value.request_index,
                            HistoryItem::ToolResult(value) => value.request_index,
                            _ => 0,
                        };
                        let response_index =
                            u32::try_from(active.response_base + u64::from(request_index))
                                .map_err(|_| {
                                    Fault::new(
                                        FaultKind::SessionInternalUnexpected,
                                        "response index exhausted",
                                    )
                                })?;
                        entries.push(HistoryEntry::Runtime {
                            turn_id: turn_id.clone(),
                            response_index,
                            item: item.clone(),
                        });
                    }
                }
            }
            let binding = Self::register_turn_save(
                &mut state,
                turn_id,
                Boundary::Complete {
                    loop_id,
                    end: offset + items.len(),
                },
                entries.len(),
            )?;
            // Accepted content, its cursor and its original save become visible
            // together. None is rolled back on I/O failure; the final report
            // supplements only the unseen suffix.
            let active = state.active.as_mut().expect("checked original Turn");
            active.bound_pending.drain(..consumed.len());
            active.accepted += items.len();
            active.consumed_inputs.extend(consumed.iter().cloned());
            for message in consumed {
                Self::record_delegated_input(&mut state, turn_id, &message);
            }
            let start = state.history.len();
            for (offset, entry) in entries.iter().enumerate() {
                state.input_provenance.record(start + offset, entry);
            }
            state.history.extend_from_slice(items);
            state
                .readable_history
                .as_mut()
                .expect("history loaded before execution")
                .extend(items.iter().map(HistoryItemData::from));
            let diagnostics = self.observe_history(&mut state, items);
            self.publish(&mut state);
            (binding, entries, diagnostics)
        };
        diagnostic::emit(diagnostics);
        self.persist_registered(binding, entries).await
    }
    /// Called in the same short owner boundary that accepts these entries. The
    /// backend append and settlement remain outside that boundary.
    fn register_turn_save(
        state: &mut SessionState,
        turn_id: &TurnId,
        boundary: Boundary,
        entry_count: usize,
    ) -> Result<SaveBinding, Fault> {
        if let Some(work) = &state.history_work {
            return Err(work.fault.clone().unwrap_or_else(|| {
                Fault::new(
                    FaultKind::SessionHistoryWriteFailed,
                    "original save is pending",
                )
            }));
        }
        let active = state
            .active
            .as_mut()
            .filter(|active| &active.info.id == turn_id)
            .ok_or_else(stale_history)?;
        let binding = SaveBinding::Turn(Binding {
            turn_id: turn_id.clone(),
            load_id: active.info.load_id.clone(),
            boundary,
        });
        if state.info.mode == StorageMode::Persistent {
            active.info.persistence = PersistenceState::Pending;
        }
        state.history_position += entry_count as u64;
        state.history_work = Some(HistorySaveWork {
            binding: binding.clone(),
            running: true,
            fault: None,
        });
        state.saving = true;
        Ok(binding)
    }
    async fn persist_registered(
        &self,
        binding: SaveBinding,
        entries: Vec<HistoryEntry>,
    ) -> Result<(), Fault> {
        let result = checked_save(async { self.backend.append(entries).await }).await;
        self.settle_save(&binding, result)
    }
    pub(super) fn register_compaction_save(
        &self,
        state: &mut SessionState,
        binding: compaction::CompactionBinding,
    ) -> SaveBinding {
        let binding = SaveBinding::Compaction(binding);
        state.history_position += 1;
        state.history_work = Some(HistorySaveWork {
            binding: binding.clone(),
            running: true,
            fault: None,
        });
        state.saving = true;
        binding
    }
    pub(super) fn settle_save(
        &self,
        binding: &SaveBinding,
        result: Result<PersistenceState, Fault>,
    ) -> Result<(), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state
            .history_work
            .as_ref()
            .is_none_or(|work| &work.binding != binding)
        {
            return Err(stale_history());
        }
        match binding {
            SaveBinding::Turn(binding)
                if state.active.as_ref().is_none_or(|active| {
                    active.info.id != binding.turn_id || active.info.load_id != binding.load_id
                }) =>
            {
                return Err(stale_history());
            }
            SaveBinding::Compaction(binding)
                if state
                    .compaction
                    .as_ref()
                    .is_none_or(|work| &work.binding != binding) =>
            {
                return Err(stale_history());
            }
            _ => {}
        }
        let result = result.and_then(|persistence| {
            if persistence
                == match self.backend.mode() {
                    StorageMode::Persistent => PersistenceState::Persisted,
                    StorageMode::Memory => PersistenceState::NotApplicable,
                }
            {
                Ok(persistence)
            } else {
                Err(Fault::new(
                    FaultKind::SessionHistoryWriteFailed,
                    "history backend did not confirm the required save",
                ))
            }
        });
        state.saving = false;
        match &result {
            Ok(_) => {
                state.history_work = None;
                if state
                    .block_reason
                    .as_ref()
                    .is_some_and(|fault| fault.kind() == Some(FaultKind::SessionHistoryWriteFailed))
                {
                    state.block_reason = state
                        .active
                        .as_ref()
                        .and_then(|active| active.tool_cleanup.fault.clone());
                }
            }
            Err(fault) => {
                let work = state.history_work.as_mut().expect("original save retained");
                work.running = false;
                work.fault = Some(fault.clone());
                state.block_reason = Some(fault.clone());
            }
        }
        let mut compaction_records = Vec::new();
        match binding {
            SaveBinding::Turn(binding) => {
                let active = state.active.as_mut().expect("checked Turn");
                match &result {
                    Ok(persistence) => {
                        active.info.persistence = if *persistence == PersistenceState::Persisted
                            && active.info.state == TurnState::Finalizing
                            && binding.boundary != Boundary::Terminal
                        {
                            PersistenceState::Pending
                        } else {
                            *persistence
                        };
                        active.info.persistence_fault = None;
                    }
                    Err(fault) => {
                        active.info.persistence = PersistenceState::Failed;
                        active.info.persistence_fault = Some(fault.clone());
                    }
                }
            }
            SaveBinding::Compaction(binding) => {
                // Automatic utility persistence has its own facts, while a
                // failed boundary still blocks the owning Turn's final handoff.
                if let Some(turn_id) = binding.turn_id()
                    && let Some(active) = state
                        .active
                        .as_mut()
                        .filter(|active| &active.info.id == turn_id)
                {
                    if let Err(fault) = &result {
                        active.info.persistence = PersistenceState::Failed;
                        active.info.persistence_fault = Some(fault.clone());
                    } else {
                        active.info.persistence_fault = None;
                        active.info.persistence = if self.backend.mode() == StorageMode::Memory {
                            PersistenceState::NotApplicable
                        } else {
                            PersistenceState::Pending
                        };
                    }
                }
                compaction_records = self.settle_compaction_save(&mut state, binding, &result)?;
            }
        }
        self.publish(&mut state);
        let record = result.as_ref().err().map(|fault| {
            let (context, phase) = match binding {
                SaveBinding::Turn(binding) => (
                    minicore_host_diagnostics::Context {
                        turn_id: Some(binding.turn_id.clone()),
                        load_id: Some(binding.load_id.clone()),
                        ..diagnostic::session_context(&self.identity())
                    },
                    match binding.boundary {
                        Boundary::Complete { .. } => "append",
                        Boundary::Partial { .. } => "partial",
                        Boundary::Terminal => "terminal",
                    },
                ),
                SaveBinding::Compaction(binding) => {
                    (diagnostics::compaction_context(&self.identity(), binding), "compaction")
                }
            };
            diagnostics::history_failed(context, fault, phase, state.controller_revision)
        });
        let compaction_records = compaction_records
            .into_iter()
            .map(|record| record.published(state.controller_revision, state.runner_revision))
            .collect::<Vec<_>>();
        drop(state);
        diagnostic::emit(record);
        diagnostic::emit(compaction_records);
        result.map(|_| ())
    }
    pub(super) async fn finalize(
        &self,
        turn_id: &TurnId,
        outcome: TurnOutcome,
        fault: Option<Fault>,
    ) {
        let record = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
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
            self.terminal_claim_record(&state, newly_claimed)
        };
        diagnostic::emit(record);
        // Runtime may have dropped a tool future on cancellation. Actual tool
        // workers must still be joined, even if an original history save failed.
        if !self.finish_tool_cleanup(turn_id).await {
            return;
        }
        let save = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.history_work.is_some() {
                return;
            }
            let stopping = state.host_stopping;
            let Some(active) = state
                .active
                .as_ref()
                .filter(|active| &active.info.id == turn_id)
            else {
                return;
            };
            let outcome = active.info.outcome.expect("terminal claim");
            let save = if active.terminal_accepted {
                None
            } else {
                let entry = HistoryEntry::TurnFinished {
                    turn_id: turn_id.clone(),
                    outcome,
                    reason: active
                        .info
                        .fault
                        .as_ref()
                        .map(|fault| fault.message().to_owned())
                        .or_else(|| {
                            active
                                .report
                                .as_ref()
                                .and_then(|report| match &report.outcome {
                                    LoopOutcome::Cancelled(reason) => {
                                        Some(format!("Runtime cancellation: {reason:?}"))
                                    }
                                    LoopOutcome::Failed(failure)
                                        if outcome == TurnOutcome::Failed =>
                                    {
                                        Some(failure.diagnostic.message().as_str().to_owned())
                                    }
                                    _ => None,
                                })
                        })
                        .or_else(|| {
                            (outcome == TurnOutcome::Cancelled).then(|| {
                                if stopping {
                                    "Host shutdown".to_owned()
                                } else {
                                    "user cancellation".to_owned()
                                }
                            })
                        }),
                };
                let Ok(binding) =
                    Self::register_turn_save(&mut state, turn_id, Boundary::Terminal, 1)
                else {
                    return;
                };
                state
                    .active
                    .as_mut()
                    .expect("checked original Turn")
                    .terminal_accepted = true;
                Some((binding, vec![entry]))
            };
            self.publish(&mut state);
            save
        };
        if let Some((binding, entries)) = save
            && self.persist_registered(binding, entries).await.is_err()
        {
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
        state.completed.insert(turn_id.clone(), result);
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
    pub(super) fn start_save_continuations(
        self: &Arc<Self>,
        state: &mut SessionState,
        resume_scheduling: bool,
    ) -> Result<(), Fault> {
        Self::available(state)?;
        let metadata = Self::record_continuation_candidate(state)?;
        let history = state.history_work.is_some();
        if !history && metadata.is_none() {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "there is no original Session save to continue",
            ));
        }
        // Both readiness checks precede spawning either original operation.
        if history {
            self.start_history_continuation(state)?;
        }
        if resume_scheduling {
            state.resume_after_saves = Some(state.pause_generation);
        }
        if let Some(candidate) = metadata {
            self.start_record_continuation(state, candidate);
        }
        Ok(())
    }
    pub(super) fn finish_save_resume(&self, state: &mut SessionState) -> bool {
        if state.history_work.is_some()
            || state.record_work.is_some()
            || state.saving
            || state
                .active
                .as_ref()
                .is_some_and(|active| active.info.state == TurnState::Finalizing)
        {
            return false;
        }
        let Some(pause_generation) = state.resume_after_saves.take() else {
            return false;
        };
        if state.pause_generation == pause_generation
            && state.lifecycle == SessionLifecycle::Open
            && !state.host_stopping
            && !state.releasing
            && !state.released
            && !state.reclaimed
            && self.access.execution_allowed_owned()
            && state.block_reason.is_none()
        {
            state.paused = false;
            state.failure_paused = false;
            state.unloaded_with_input = false;
            true
        } else {
            false
        }
    }
    fn start_history_continuation(self: &Arc<Self>, state: &mut SessionState) -> Result<(), Fault> {
        if state.reclaimed || state.released || state.releasing {
            return Err(stale_history());
        }
        let work = state.history_work.as_ref().ok_or_else(|| {
            Fault::new(
                FaultKind::SessionBusy,
                "there is no original history save to continue",
            )
        })?;
        if work.running || state.saving || state.driver_running {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "original execution or save is still returning",
            ));
        }
        let binding = work.binding.clone();
        let turn_id = match &binding {
            SaveBinding::Turn(binding) => {
                let active = state
                    .active
                    .as_ref()
                    .filter(|active| {
                        active.info.id == binding.turn_id && active.info.load_id == binding.load_id
                    })
                    .ok_or_else(stale_history)?;
                if active.info.state != TurnState::Finalizing {
                    return Err(stale_history());
                }
                Some(binding.turn_id.clone())
            }
            SaveBinding::Compaction(binding) => {
                if state
                    .compaction
                    .as_ref()
                    .is_none_or(|work| &work.binding != binding)
                {
                    return Err(stale_history());
                }
                binding.turn_id().cloned()
            }
        };
        let terminal = turn_id
            .as_ref()
            .map(|id| {
                let active = state
                    .active
                    .as_ref()
                    .filter(|active| {
                        &active.info.id == id && active.info.state == TurnState::Finalizing
                    })
                    .ok_or_else(stale_history)?;
                Ok((
                    id.clone(),
                    active.info.outcome.ok_or_else(stale_history)?,
                    active.info.fault.clone(),
                    active.report.clone(),
                ))
            })
            .transpose()?;
        state.history_work.as_mut().expect("original save").running = true;
        state.saving = true;
        let previous = state.history_task.take();
        let compaction = state.compaction_task.take();
        let driver = state.driver_task.take();
        let owner = Arc::clone(self);
        state.history_task = Some(state.executor.as_ref().expect("checked runtime").spawn(
            async move {
                // No active driver is allowed at admission. In particular, never
                // wait on a Runtime join whose terminal callback awaits this save.
                for task in [previous, compaction, driver].into_iter().flatten() {
                    if task.await.is_err() {
                        let _ = owner.settle_save(
                            &binding,
                            Err(Fault::new(
                                FaultKind::SessionHistoryWriteFailed,
                                "original execution owner did not retire normally",
                            )),
                        );
                        return;
                    }
                }
                let result = checked_save(async { owner.backend.reconcile().await }).await;
                if owner.settle_save(&binding, result).is_err() {
                    return;
                }
                if let Some((turn_id, outcome, fault, report)) = terminal {
                    if let Some(report) = report
                        && owner.reconcile_report(&turn_id, &report).await.is_err()
                    {
                        return;
                    }
                    owner.finalize(&turn_id, outcome, fault).await;
                }
                {
                    let mut state = owner.state.lock().expect("Session mutex poisoned");
                    if owner.finish_save_resume(&mut state) {
                        owner.publish(&mut state);
                    }
                }
                owner.handoff_delegation_owned();
                owner.wake();
            },
        ));
        self.publish(state);
        Ok(())
    }
}
pub(super) async fn checked_save(
    future: impl Future<Output = Result<PersistenceState, Fault>>,
) -> Result<PersistenceState, Fault> {
    AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "history backend panicked before confirming its save",
            ))
        })
        .map_err(|fault| {
            if fault.kind() == Some(FaultKind::SessionHistoryWriteFailed) {
                fault
            } else {
                Fault::new(FaultKind::SessionHistoryWriteFailed, fault.message())
            }
        })
}
fn stale_history() -> Fault {
    Fault::new(
        FaultKind::SessionInternalUnexpected,
        "history work does not match its original Turn, Load, and execution range",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stale_turn_load_or_boundary_completion_cannot_settle_another_range() {
        let fixture = super::super::tests::original_save_fixture().await;
        let original = fixture
            .session
            .state
            .lock()
            .unwrap()
            .history_work
            .as_ref()
            .unwrap()
            .binding
            .clone();
        let SaveBinding::Turn(original_turn) = &original else {
            panic!("expected Turn save")
        };
        let mut bindings = Vec::new();
        let mut wrong_turn = original_turn.clone();
        wrong_turn.turn_id = TurnId::new().unwrap();
        bindings.push(wrong_turn);
        let mut wrong_load = original_turn.clone();
        wrong_load.load_id = LoadId::new().unwrap();
        bindings.push(wrong_load);
        let mut wrong_range = original_turn.clone();
        wrong_range.boundary = Boundary::Terminal;
        bindings.push(wrong_range);
        for stale in bindings {
            assert!(
                fixture
                    .session
                    .settle_save(&SaveBinding::Turn(stale), Ok(PersistenceState::Persisted))
                    .is_err()
            );
            let state = fixture.session.state.lock().unwrap();
            assert_eq!(state.history_work.as_ref().unwrap().binding, original);
            assert_eq!(
                state.active.as_ref().unwrap().info.persistence,
                PersistenceState::Failed
            );
            assert!(state.completed.is_empty());
        }
    }
}
