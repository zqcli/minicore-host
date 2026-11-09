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
struct Binding {
    turn_id: TurnId,
    load_id: LoadId,
    boundary: Boundary,
}
pub(super) struct HistorySaveWork {
    binding: Binding,
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
        let entries = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let active = state.active.as_mut().ok_or_else(stale_history)?;
            if &active.info.id != turn_id || active.loop_id != Some(report.loop_id) {
                return Err(stale_history());
            }
            if let Some(partial) = &report.partial {
                if active.partial_accepted {
                    Vec::new()
                } else {
                    let response_index = active.response_base + u64::from(partial.request_index);
                    let encoded_index =
                        u32::try_from(response_index).map_err(|_| stale_history())?;
                    active.partials.push(PartialResult {
                        response_index,
                        partial: partial.clone(),
                    });
                    active.partial_accepted = true;
                    vec![HistoryEntry::Partial {
                        turn_id: turn_id.clone(),
                        loop_id: report.loop_id,
                        response_index: encoded_index,
                        partial: partial.clone(),
                    }]
                }
            } else {
                Vec::new()
            }
        };
        if !entries.is_empty() {
            self.save(
                turn_id,
                Boundary::Partial {
                    loop_id: report.loop_id,
                },
                entries,
            )
            .await?;
        }
        Ok(())
    }
    pub(super) async fn accept_history(
        &self,
        turn_id: &TurnId,
        loop_id: LoopId,
        offset: usize,
        items: &[HistoryItem],
    ) -> Result<(), Fault> {
        let entries = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let active = state.active.as_mut().ok_or_else(|| {
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
            for item in items {
                match item {
                    HistoryItem::User(user) => {
                        let input = active.bound_pending.pop_front().ok_or_else(|| {
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
                        entries.push(HistoryEntry::Input {
                            turn_id: turn_id.clone(),
                            message_id: input.message.message_id,
                            source: input.source,
                            mode: input.message.mode,
                            in_reply_to: input.message.in_reply_to,
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
            // Accepted cursor is advanced before I/O and never rolled back on a
            // save failure. The final report supplements only the unseen suffix.
            active.accepted += items.len();
            state.history.extend_from_slice(items);
            self.publish(&mut state);
            entries
        };
        self.save(
            turn_id,
            Boundary::Complete {
                loop_id,
                end: offset + items.len(),
            },
            entries,
        )
        .await
    }
    async fn save(
        &self,
        turn_id: &TurnId,
        boundary: Boundary,
        entries: Vec<HistoryEntry>,
    ) -> Result<(), Fault> {
        let binding = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
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
            let binding = Binding {
                turn_id: turn_id.clone(),
                load_id: active.info.load_id.clone(),
                boundary,
            };
            if self.backend.mode() == StorageMode::Persistent {
                active.info.persistence = PersistenceState::Pending;
            }
            state.history_work = Some(HistorySaveWork {
                binding: binding.clone(),
                running: true,
                fault: None,
            });
            state.saving = true;
            self.publish(&mut state);
            binding
        };
        let result = checked_save(self.backend.append(entries)).await;
        self.settle_save(&binding, result)
    }
    fn settle_save(
        &self,
        binding: &Binding,
        result: Result<PersistenceState, Fault>,
    ) -> Result<(), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state
            .history_work
            .as_ref()
            .is_none_or(|work| &work.binding != binding)
            || state.active.as_ref().is_none_or(|active| {
                active.info.id != binding.turn_id || active.info.load_id != binding.load_id
            })
        {
            return Err(stale_history());
        }
        state.saving = false;
        let active = state.active.as_mut().expect("checked original Turn");
        let result = match result {
            Ok(persistence)
                if persistence
                    == match self.backend.mode() {
                        StorageMode::Persistent => PersistenceState::Persisted,
                        StorageMode::Memory => PersistenceState::NotApplicable,
                    } =>
            {
                active.info.persistence = persistence;
                active.info.persistence_fault = None;
                state.history_work = None;
                if state
                    .block_reason
                    .as_ref()
                    .is_some_and(|fault| fault.kind() == FaultKind::SessionHistoryWriteFailed)
                {
                    state.block_reason = None;
                }
                Ok(())
            }
            other => {
                let fault = match other {
                    Err(fault) => fault,
                    Ok(_) => Fault::new(
                        FaultKind::SessionHistoryWriteFailed,
                        "history backend did not confirm the required save",
                    ),
                };
                active.info.persistence = PersistenceState::Failed;
                active.info.persistence_fault = Some(fault.clone());
                let work = state.history_work.as_mut().expect("original save retained");
                work.running = false;
                work.fault = Some(fault.clone());
                state.block_reason = Some(fault.clone());
                Err(fault)
            }
        };
        self.publish(&mut state);
        result
    }
    pub(super) async fn finalize(
        &self,
        turn_id: &TurnId,
        outcome: TurnOutcome,
        fault: Option<Fault>,
    ) {
        let entry = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            if state.history_work.is_some() {
                return;
            }
            let Some(active) = state
                .active
                .as_mut()
                .filter(|active| &active.info.id == turn_id)
            else {
                return;
            };
            // A later save/cancel/force cannot revise a previously claimed outcome.
            if active.info.outcome.is_none() {
                active.info.outcome = Some(outcome);
                active.info.fault = fault;
            }
            active.info.state = TurnState::Finalizing;
            active.handle = None;
            let outcome = active.info.outcome.expect("terminal claim");
            let entry = if active.terminal_accepted {
                None
            } else {
                active.terminal_accepted = true;
                Some(HistoryEntry::TurnFinished {
                    turn_id: turn_id.clone(),
                    outcome,
                    reason: active
                        .info
                        .fault
                        .as_ref()
                        .map(|fault| fault.message().to_owned()),
                })
            };
            if outcome == TurnOutcome::Failed {
                state.failure_paused = true;
            }
            self.publish(&mut state);
            entry
        };
        if let Some(entry) = entry
            && self
                .save(turn_id, Boundary::Terminal, vec![entry])
                .await
                .is_err()
        {
            return;
        }
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.history_work.is_some()
            || state.active.as_ref().is_none_or(|active| {
                &active.info.id != turn_id
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
        state.completion_order.push(turn_id.clone());
        state.completed.insert(turn_id.clone(), result);
        if state.lifecycle == SessionLifecycle::Closing && state.info.mode == StorageMode::Memory {
            state.lifecycle = SessionLifecycle::Closed;
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
        self.publish(&mut state);
    }
    /// Only the direct Agent's already-accepted saving workflow may call this
    /// without a connection. No new Runtime work or implicit retries are started.
    pub(crate) fn continue_history_owned(self: &Arc<Self>) -> Result<(), Fault> {
        self.ensure_executor()?;
        let mut state = self.state.lock().expect("Session mutex poisoned");
        self.start_history_continuation(&mut state, false)
    }
    pub(super) fn start_history_continuation(
        self: &Arc<Self>,
        state: &mut SessionState,
        resume_scheduling: bool,
    ) -> Result<(), Fault> {
        if state.reclaimed || state.released || state.releasing {
            return Err(stale_history());
        }
        let active = state.active.as_ref().ok_or_else(stale_history)?;
        let work = state.history_work.as_ref().ok_or_else(|| {
            Fault::new(
                FaultKind::SessionBusy,
                "there is no original history save to continue",
            )
        })?;
        if work.running
            || state.driver_running
            || state.saving
            || active.info.state != TurnState::Finalizing
        {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "original execution or save is still returning",
            ));
        }
        let binding = work.binding.clone();
        if binding.turn_id != active.info.id || binding.load_id != active.info.load_id {
            return Err(stale_history());
        }
        let outcome = active.info.outcome.ok_or_else(stale_history)?;
        let fault = active.info.fault.clone();
        let report = active.report.clone();
        let pause_generation = state.pause_generation;
        state
            .history_work
            .as_mut()
            .expect("checked original save")
            .running = true;
        state.saving = true;
        let previous = state.history_task.take();
        let owner = Arc::clone(self);
        state.history_task = Some(state.executor.as_ref().expect("checked runtime").spawn(
            async move {
                if let Some(previous) = previous {
                    let _ = previous.await;
                }
                let result = checked_save(owner.backend.reconcile()).await;
                if owner.settle_save(&binding, result).is_err() {
                    return;
                }
                if let Some(report) = report
                    && owner
                        .reconcile_report(&binding.turn_id, &report)
                        .await
                        .is_err()
                {
                    return;
                }
                owner.finalize(&binding.turn_id, outcome, fault).await;
                {
                    let mut state = owner.state.lock().expect("Session mutex poisoned");
                    if resume_scheduling
                        && state.pause_generation == pause_generation
                        && state.completed.contains_key(&binding.turn_id)
                        && state.lifecycle == SessionLifecycle::Open
                        && !state.host_stopping
                    {
                        state.paused = false;
                        state.failure_paused = false;
                        owner.publish(&mut state);
                    }
                }
                owner.wake();
            },
        ));
        self.publish(state);
        Ok(())
    }
}
async fn checked_save(future: SaveFuture<'_>) -> Result<PersistenceState, Fault> {
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
            if fault.kind() == FaultKind::SessionHistoryWriteFailed {
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
