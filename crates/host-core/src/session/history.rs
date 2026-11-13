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
    pub(super) running: bool,
    pub(super) fault: Option<Fault>,
}
impl SessionController {
    /// Accept only this newly returned delta, then save the original unsaved
    /// native range. Input records were authored directly at queue consumption.
    pub(super) async fn save_history(
        self: &Arc<Self>,
        turn_id: &TurnId,
        items: &[HistoryItem],
    ) -> Result<(), Fault> {
        self.wait_compaction_boundary().await?;
        let (binding, entries, diagnostics) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let active = state
                .active
                .as_ref()
                .filter(|a| &a.info.id == turn_id)
                .ok_or_else(stale_history)?;
            let start = active.save_cursor;
            let loop_id = active.loop_id;
            if let Some(work) = &state.history_work {
                return Err(work.fault.clone().unwrap_or_else(|| {
                    Fault::new(
                        FaultKind::SessionHistoryWriteFailed,
                        "original save is pending",
                    )
                }));
            }
            if start == state.history.len() && items.is_empty() {
                return Ok(());
            }
            let mut entries = state.history[start..]
                .iter()
                .enumerate()
                .map(|(offset, item)| state.input_provenance.entry(start + offset, turn_id, item))
                .collect::<Result<Vec<_>, _>>()?;
            for item in items {
                let index = match item {
                    HistoryItem::Assistant(value) if value.loop_id == loop_id => {
                        value.request_index
                    }
                    HistoryItem::ToolResult(value) if value.loop_id == loop_id => {
                        value.request_index
                    }
                    _ => return Err(stale_history()),
                };
                entries.push(HistoryEntry::Runtime {
                    turn_id: turn_id.clone(),
                    response_index: index,
                    item: item.clone(),
                });
            }
            let accepted_at = Timestamp::now();
            let records = entries
                .into_iter()
                .enumerate()
                .map(|(offset, entry)| {
                    let position = state.history_position + offset as u64;
                    let recorded_at = state
                        .readable_history
                        .as_ref()
                        .and_then(|rows| rows.get(position as usize))
                        .map_or(accepted_at, |row| row.recorded_at);
                    HistoryRecord::at(self.id.clone(), position, recorded_at, entry)
                })
                .collect::<Vec<_>>();
            let end = state.history.len() + items.len();
            let binding = Self::register_turn_save(
                &mut state,
                turn_id,
                Boundary::Complete { loop_id, end },
                records.len(),
            )?;
            Arc::make_mut(&mut state.history).extend_from_slice(items);
            state.active.as_mut().expect("checked Turn").save_cursor = end;
            // Inputs became readable at consumption; only the returned delta
            // is newly readable here. Save confirmation remains independent.
            let readable = state.readable_history.as_mut().expect("loaded history");
            let existing = readable.len() as u64;
            readable.extend(
                records
                    .iter()
                    .filter(|record| record.position >= existing)
                    .map(HistoryRecordData::from_record),
            );
            for record in &records {
                state.info.last_activity_at = state.info.last_activity_at.max(record.recorded_at);
            }
            let diagnostics = self.observe_history(&mut state, &records);
            self.checkpoint_activity_locked(&mut state);
            self.publish(&mut state);
            (binding, records, diagnostics)
        };
        diagnostic::emit(diagnostics);
        self.persist_registered(binding, entries).await
    }

    /// The one failed model call's real partial survives a prior save failure.
    /// Resuming the original range reaches this method without rerunning work.
    pub(super) async fn save_partial(self: &Arc<Self>, turn_id: &TurnId) -> Result<(), Fault> {
        let (binding, entries) = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let active = state
                .active
                .as_ref()
                .filter(|a| &a.info.id == turn_id)
                .ok_or_else(stale_history)?;
            let Some(partial) = active
                .pending_partial
                .as_ref()
                .filter(|_| !active.partial_accepted)
            else {
                return Ok(());
            };
            let partial = partial.clone();
            let loop_id = active.loop_id;
            let response_index = u64::from(partial.request_index);
            let recorded_at = Timestamp::now();
            let entries = vec![HistoryRecord::at(
                self.id.clone(),
                state.history_position,
                recorded_at,
                HistoryEntry::Partial {
                    turn_id: turn_id.clone(),
                    loop_id,
                    response_index: partial.request_index,
                    partial: partial.clone(),
                },
            )];
            let binding =
                Self::register_turn_save(&mut state, turn_id, Boundary::Partial { loop_id }, 1)?;
            let active = state.active.as_mut().expect("checked Turn");
            active.partials.push(PartialResult {
                response_index,
                partial,
            });
            active.partial_accepted = true;
            active.pending_partial = None;
            state
                .readable_history
                .as_mut()
                .expect("loaded history")
                .extend(entries.iter().map(HistoryRecordData::from_record));
            state.info.last_activity_at = state.info.last_activity_at.max(recorded_at);
            self.checkpoint_activity_locked(&mut state);
            self.publish(&mut state);
            (binding, entries)
        };
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
        Ok(binding)
    }
    async fn persist_registered(
        self: &Arc<Self>,
        binding: SaveBinding,
        entries: Vec<HistoryRecord>,
    ) -> Result<(), Fault> {
        let result = checked_save(async { self.backend.append(entries).await }).await;
        self.settle_save(&binding, result)
    }
    pub(super) fn register_compaction_save(
        &self,
        state: &mut SessionState,
        binding: compaction::CompactionBinding,
        entry_count: usize,
    ) -> SaveBinding {
        let binding = SaveBinding::Compaction(binding);
        state.history_position += entry_count as u64;
        state.history_work = Some(HistorySaveWork {
            binding: binding.clone(),
            running: true,
            fault: None,
        });
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
        let previous = diagnostics::Schedule::capture(&state);
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
                SaveBinding::Compaction(binding) => (
                    diagnostics::compaction_context(&self.identity(), binding),
                    "compaction",
                ),
            };
            diagnostics::history_failed(context, fault, phase, state.controller_revision)
        });
        let compaction_records = compaction_records
            .into_iter()
            .map(|record| record.published(state.controller_revision, state.runner_revision))
            .collect::<Vec<_>>();
        let operation = match binding {
            SaveBinding::Compaction(compaction::CompactionBinding::Manual {
                operation_id, ..
            }) => Some(operation_id),
            _ => None,
        };
        let scheduling =
            self.scheduling_record(&state, previous, "history_save_finished", operation);
        drop(state);
        diagnostic::emit(record);
        diagnostic::emit(compaction_records);
        diagnostic::emit(scheduling);
        result.map(|_| ())
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
        if work.running || state.driver_running {
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
                ))
            })
            .transpose()?;
        state.history_work.as_mut().expect("original save").running = true;
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
                if let Some((turn_id, outcome, fault)) = terminal {
                    owner.finish_turn(&turn_id, outcome, fault).await;
                }
                let scheduling = {
                    let mut state = owner.state.lock().expect("Session mutex poisoned");
                    let previous = diagnostics::Schedule::capture(&state);
                    if owner.finish_save_resume(&mut state) {
                        owner.publish(&mut state);
                        owner.scheduling_record(&state, previous, "save_resume_finished", None)
                    } else {
                        None
                    }
                };
                diagnostic::emit(scheduling);
                owner.handoff_delegation_owned();
                owner.wake();
            },
        ));
        self.publish(state);
        Ok(())
    }
    /// Accept and confirm the original terminal record before result publication.
    /// False keeps finalization with its current owner and outstanding save.
    pub(super) async fn save_terminal(self: &Arc<Self>, turn_id: &TurnId) -> bool {
        let save = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.history_work.is_some() {
                return false;
            }
            let stopping = state.host_stopping;
            let Some(active) = state
                .active
                .as_ref()
                .filter(|active| &active.info.id == turn_id)
            else {
                return false;
            };
            let outcome = active.info.outcome.expect("terminal claim");
            let save = if active.terminal_accepted {
                None
            } else {
                let recorded_at = Timestamp::now();
                let entry = HistoryEntry::TurnFinished {
                    finished_at: recorded_at,
                    requests: active.info.requests,
                    tool_rounds: active.info.tool_rounds,
                    model_attempts: active.info.model_attempts.clone(),
                    turn_id: turn_id.clone(),
                    outcome,
                    reason: active
                        .info
                        .fault
                        .as_ref()
                        .map(|fault| fault.message().to_owned())
                        .or_else(|| {
                            (outcome == TurnOutcome::Cancelled).then(|| {
                                if active.deadline_expired {
                                    "Turn deadline expired".to_owned()
                                } else if stopping {
                                    "Host Shutdown".to_owned()
                                } else {
                                    "user cancellation".to_owned()
                                }
                            })
                        }),
                };
                let entry =
                    HistoryRecord::at(self.id.clone(), state.history_position, recorded_at, entry);
                let Ok(binding) =
                    Self::register_turn_save(&mut state, turn_id, Boundary::Terminal, 1)
                else {
                    return false;
                };
                state
                    .active
                    .as_mut()
                    .expect("checked original Turn")
                    .terminal_accepted = true;
                state
                    .readable_history
                    .as_mut()
                    .expect("loaded history")
                    .push(HistoryRecordData::from_record(&entry));
                state.info.last_activity_at = state.info.last_activity_at.max(recorded_at);
                Some((binding, vec![entry]))
            };
            self.checkpoint_activity_locked(&mut state);
            self.publish(&mut state);
            save
        };
        if let Some((binding, entries)) = save
            && self.persist_registered(binding, entries).await.is_err()
        {
            return false;
        }
        true
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
