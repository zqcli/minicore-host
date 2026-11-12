use super::*;
use crate::diagnostics::{self as diagnostic, Record};
use crate::observation::*;
use minicore_host_diagnostics::{EventData, Scope};
use minicore_host_runtime::{
    model::{AssistantPart, ModelCallEvent},
    tools::ToolBatchEvent,
};
use tracing::Level;

#[derive(Default)]
pub(super) struct TurnObservation {
    // Retained with the original Turn, never projected from a later Runner.
    terminal_state: Option<TurnStateData>,
    pub(super) interactions: BTreeMap<minicore_host_runtime::InteractionId, InteractionRecord>,
    pub(super) models: BTreeMap<u64, BTreeMap<String, Body>>,
    pub(super) tools: BTreeMap<ToolCallId, ToolRecord>,
}
pub(super) struct InteractionRecord {
    pub(super) data: PendingInteractionData,
    pub(super) resolved: bool,
}
impl TurnObservation {
    pub(super) fn interaction(
        &mut self,
        pending: &minicore_host_runtime::interaction::PendingInteraction,
    ) -> &mut InteractionRecord {
        self.interactions
            .entry(pending.interaction_id)
            .or_insert_with(|| InteractionRecord {
                data: PendingInteractionData::from_runtime(
                    pending,
                    InteractionId::new().expect("Host interaction identity generation"),
                ),
                resolved: false,
            })
    }
}
#[derive(Default)]
pub(super) struct Body {
    pub(super) live: String,
    // A binary channel has only a real contiguous UTF-8 prefix. Never imply
    // that a request crossing its end returned the complete process stream.
    pub(super) non_utf8: bool,
    pub(super) complete: Option<String>,
}
impl Body {
    pub(super) fn text(&self) -> &str {
        self.complete.as_deref().unwrap_or(&self.live)
    }
}
pub(super) struct ToolRecord {
    pub(super) data: ToolCallData,
    pub(super) output: BTreeMap<String, Body>,
    // The same cleanup-scoped admission held by the actual worker. Native
    // history may arrive earlier, but public finished requires this to close.
    pub(super) owned_output: Option<Arc<AtomicBool>>,
}

impl SessionController {
    pub(super) fn identity(&self) -> SessionIdentity {
        SessionIdentity {
            host_id: self.host_id.clone(),
            host_incarnation: self.incarnation.clone(),
            agent_id: self.agent_id.clone(),
            session_id: self.id.clone(),
        }
    }
    pub(super) fn turn_identity(&self, info: &TurnInfo) -> TurnIdentity {
        TurnIdentity {
            session: self.identity(),
            turn_id: info.id.clone(),
            load_id: Some(info.load_id.clone()),
        }
    }
    pub(super) fn turn_state(info: &TurnInfo, revision: u64) -> TurnStateData {
        TurnStateData {
            state: info.state,
            runner_revision: revision,
            config_revision: info.config_revision,
            outcome: info.outcome,
            persistence: info.persistence,
            requests: info.requests,
            tool_rounds: info.tool_rounds,
            phase: info.phase,
            model_retry: info.model_retry.clone(),
            model_attempts: info.model_attempts.clone(),
            fault: info.fault.clone(),
            persistence_fault: info.persistence_fault.clone(),
            cleanup_faults: info.cleanup_faults.clone(),
        }
    }
    pub(super) fn retained_turn_state(state: &SessionState, info: &TurnInfo) -> TurnStateData {
        if info.state == TurnState::Finished {
            state
                .output
                .get(&info.id)
                .and_then(|turn| turn.terminal_state.as_ref())
                .expect("finished Turn retains its terminal observation")
                .clone()
        } else {
            Self::turn_state(info, state.runner_revision)
        }
    }
    pub(super) fn publish_observation(&self, state: &mut SessionState, snapshot: &SessionSnapshot) {
        if !state.published {
            return;
        }
        self.access
            .observation
            .publish(ObservationEvent::State(StateEvent::Session {
                source: self.identity(),
                data: snapshot.data.clone(),
            }));
        if let Some(active) = &state.active {
            self.access
                .observation
                .publish(ObservationEvent::State(StateEvent::Turn {
                    source: self.turn_identity(&active.info),
                    data: Self::turn_state(&active.info, state.runner_revision),
                }));
        }
        if state.lifecycle == SessionLifecycle::Closed && !state.closed_published {
            state.closed_published = true;
            self.access.observation.publish(ObservationEvent::Activity(
                ActivityEvent::SessionClosed {
                    source: self.identity(),
                    data: snapshot.data.clone(),
                },
            ));
        }
    }
    pub(crate) fn publish_initial(&self) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.published = true;
        self.publish(&mut state);
    }
    pub(super) fn observe_message(
        &self,
        state: &SessionState,
        record: &MessageRecord,
        turn: Option<&TurnInfo>,
    ) -> Record {
        let data = MessageEventData {
            text: record.input.text.clone(),
            mode: record.input.mode,
            source: record.source.clone(),
            queue: record.queue,
            order: record.order,
            in_reply_to: record.input.in_reply_to.clone(),
            controller_revision: state.controller_revision,
        };
        let message_id = record.input.message_id.clone();
        let event = match turn {
            Some(info) => ActivityEvent::MessageBound {
                source: self.turn_identity(info),
                message_id,
                data,
            },
            None => ActivityEvent::MessageAccepted {
                source: self.identity(),
                message_id,
                data,
            },
        };
        self.access
            .observation
            .publish(ObservationEvent::Activity(event));
        let context = turn.map_or_else(
            || diagnostic::session_context(&self.identity()),
            |info| diagnostic::turn_context(&self.turn_identity(info)),
        );
        diagnostics::input(
            context,
            record,
            state.controller_revision,
            state.next_step.len(),
            state.next_turn.len(),
            turn.is_some(),
        )
    }
    pub(super) fn observe_bound(&self, state: &SessionState, batch: &[QueuedInput]) -> Vec<Record> {
        let mut records = Vec::new();
        if let Some(active) = &state.active {
            for input in batch {
                if let Some(record) = state.messages.iter().find(|record| {
                    record.input.message_id == input.message.message_id
                        && record.queue == input.queue
                        && record.order == input.order
                }) {
                    records.push(self.observe_message(state, record, Some(&active.info)));
                }
            }
        }
        records
    }
    pub(super) fn observe_history(
        &self,
        state: &mut SessionState,
        items: &[HistoryItem],
    ) -> Vec<Record> {
        let mut diagnostics = Vec::new();
        let active = state.active.as_ref().expect("accepted active Turn");
        let source = self.turn_identity(&active.info);
        let revision = state.runner_revision;
        let observation = state.output.entry(active.info.id.clone()).or_default();
        for item in items {
            match item {
                HistoryItem::Assistant(result) => {
                    let response_index = u64::from(result.request_index);
                    let channels = observation.models.entry(response_index).or_default();
                    let mut text = String::new();
                    let mut reasoning = String::new();
                    for part in &result.content {
                        match part {
                            AssistantPart::Text(value) => text.push_str(value),
                            AssistantPart::Reasoning(value) => {
                                if let Some(value) = value.text() {
                                    reasoning.push_str(value);
                                }
                            }
                            AssistantPart::ToolCall(call) => {
                                let id = ToolCallId(call.tool_call_id().as_str().to_owned());
                                let record = ToolRecord {
                                    data: ToolCallData {
                                        state: ToolCallStateData {
                                            runner_revision: revision,
                                            response_index,
                                            name: call.name().as_str().to_owned(),
                                            state: ToolCallState::Pending,
                                            progress: None,
                                            delegations: Vec::new(),
                                            outcome: None,
                                        },
                                        arguments: call.arguments().clone(),
                                        result: None,
                                    },
                                    output: BTreeMap::new(),
                                    owned_output: None,
                                };
                                self.access.observation.publish(ObservationEvent::State(
                                    StateEvent::Toolcall {
                                        source: ToolCallIdentity {
                                            turn: source.clone(),
                                            tool_call_id: id.clone(),
                                        },
                                        data: record.data.state.clone(),
                                    },
                                ));
                                observation.tools.insert(id, record);
                            }
                        }
                    }
                    channels.entry("text".into()).or_default().complete = Some(text);
                    channels.entry("reasoning".into()).or_default().complete = Some(reasoning);
                    self.access.observation.publish(ObservationEvent::Activity(
                        ActivityEvent::ModelFinished {
                            source: ModelOutputIdentity {
                                turn: source.clone(),
                                response_index,
                            },
                            data: ModelFinishedData {
                                result: result.into(),
                                output_positions: positions(channels),
                            },
                        },
                    ));
                }
                HistoryItem::ToolResult(result) => {
                    let id = ToolCallId(result.call_id.as_str().to_owned());
                    if let Some(record) = observation.tools.get_mut(&id) {
                        // Retain the complete native result immediately. Its
                        // history/save handoff never waits for observer timing
                        // or for a cancelled tool's actual cleanup tail.
                        record.data.result = Some(result.into());
                        diagnostics.extend(self.publish_tool_finished(
                            ToolCallIdentity {
                                turn: source.clone(),
                                tool_call_id: id,
                            },
                            record,
                            revision,
                        ));
                    }
                }
                _ => {}
            }
        }
        diagnostics
    }
    pub(super) fn publish_tool_finished(
        &self,
        source: ToolCallIdentity,
        record: &mut ToolRecord,
        revision: u64,
    ) -> Option<Record> {
        if record.data.state.state == ToolCallState::Finished
            || record
                .owned_output
                .as_ref()
                .is_some_and(|live| live.load(Ordering::SeqCst))
        {
            return None;
        }
        let result = record.data.result.clone()?;
        record.data.state.state = ToolCallState::Finished;
        record.data.state.outcome = Some(result.outcome);
        record.data.state.runner_revision = revision;
        let diagnostic = diagnostics::tool_finished(&source, &record.data.state, result.outcome);
        self.access
            .observation
            .publish(ObservationEvent::State(StateEvent::Toolcall {
                source: source.clone(),
                data: record.data.state.clone(),
            }));
        self.access
            .observation
            .publish(ObservationEvent::Activity(ActivityEvent::ToolFinished {
                source,
                data: ToolFinishedData {
                    result,
                    output_positions: positions(&record.output),
                },
            }));
        Some(diagnostic)
    }

    pub(super) fn publish_cleaned_tool_results(
        &self,
        state: &mut SessionState,
        turn_id: &TurnId,
    ) -> Vec<Record> {
        let mut diagnostics = Vec::new();
        let source = self.turn_identity(&state.active.as_ref().expect("owned active Turn").info);
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let revision = state.runner_revision;
        if let Some(turn) = state.output.get_mut(turn_id) {
            for (tool_call_id, record) in &mut turn.tools {
                diagnostics.extend(self.publish_tool_finished(
                    ToolCallIdentity {
                        turn: source.clone(),
                        tool_call_id: tool_call_id.clone(),
                    },
                    record,
                    revision,
                ));
            }
        }
        diagnostics
    }

    pub(super) fn begin_model_request(
        &self,
        turn_id: &TurnId,
        request_index: u32,
        deadline: Option<tokio::time::Instant>,
    ) -> bool {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if self.stop_turn_locked(&mut state, turn_id, deadline) {
            self.publish(&mut state);
            return false;
        }
        // The final input check and request admission are one atomic boundary.
        if self.schedulable(&state) && !state.next_step.is_empty() {
            return false;
        }
        let active = state.active.as_mut().expect("owned Turn");
        active.info.requests += 1;
        active.info.phase = Some(TurnPhase::Model);
        active.info.model_retry = None;
        let loop_id = active.loop_id;
        let source = self.turn_identity(&active.info);
        let channels = state
            .output
            .entry(turn_id.clone())
            .or_default()
            .models
            .entry(u64::from(request_index))
            .or_default();
        channels.entry("text".into()).or_default();
        channels.entry("reasoning".into()).or_default();
        self.publish(&mut state);
        drop(state);
        Record::new(
            diagnostic::event(
                Level::DEBUG,
                Scope::Runtime,
                "session.runtime",
                "model.started",
                "Session issued a model request",
                &["response_index", "loop_id", "request_index"],
            ),
            EventData::default()
                .context(diagnostic::turn_context(&source))
                .attribute("response_index", u64::from(request_index))
                .attribute("loop_id", loop_id.to_string())
                .attribute("request_index", u64::from(request_index)),
        )
        .emit();
        true
    }
    pub(super) fn observe_model_call(
        &self,
        turn_id: &TurnId,
        request_index: u32,
        event: ModelCallEvent,
    ) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let Some(active) = state
            .active
            .as_mut()
            .filter(|active| &active.info.id == turn_id)
        else {
            return;
        };
        let source = self.turn_identity(&active.info);
        let loop_id = active.loop_id;
        let response_index = u64::from(request_index);
        let mut diagnostic = None;
        match event {
            ModelCallEvent::AttemptStarted(attempt) => {
                active
                    .info
                    .model_attempts
                    .resize(request_index as usize + 1, 0);
                active.info.model_attempts[request_index as usize] = attempt;
                if let Some(retry) = &mut active.info.model_retry {
                    retry.phase = ModelRetryPhase::Requesting;
                }
                self.publish(&mut state);
            }
            ModelCallEvent::RetryWaiting(retry) => {
                diagnostic = Some(diagnostics::model_retry(
                    &source,
                    loop_id,
                    request_index,
                    &retry,
                ));
                active.info.model_retry = Some(ModelRetryProgress {
                    response_index,
                    attempt: retry.attempt,
                    max_attempts: retry.max_attempts,
                    fault: runtime_model_fault(&retry.error),
                    phase: ModelRetryPhase::Waiting {
                        retry_at: retry.retry_at.into(),
                    },
                });
                self.publish(&mut state);
            }
            ModelCallEvent::TextDelta(_) | ModelCallEvent::ReasoningDelta(_) => {
                let (channel, delta) = match event {
                    ModelCallEvent::TextDelta(delta) => ("text", delta),
                    ModelCallEvent::ReasoningDelta(delta) => ("reasoning", delta),
                    _ => unreachable!(),
                };
                let body = state
                    .output
                    .entry(turn_id.clone())
                    .or_default()
                    .models
                    .entry(response_index)
                    .or_default()
                    .entry(channel.into())
                    .or_default();
                let start = body.live.len() as u64;
                body.live.push_str(delta.as_str());
                self.access.observation.publish(ObservationEvent::Activity(
                    ActivityEvent::ModelOutput {
                        source: ModelOutputIdentity {
                            turn: source,
                            response_index,
                        },
                        data: OutputDelta {
                            channel: channel.into(),
                            start,
                            end: body.live.len() as u64,
                            text: delta.as_str().to_owned(),
                        },
                    },
                ));
            }
        }
        drop(state);
        diagnostic::emit(diagnostic);
    }
    pub(super) fn observe_tool_event(&self, turn_id: &TurnId, event: ToolBatchEvent) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let Some(active) = state
            .active
            .as_ref()
            .filter(|active| &active.info.id == turn_id)
        else {
            return;
        };
        let source = self.turn_identity(&active.info);
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let revision = state.runner_revision;
        let output = state.output.entry(turn_id.clone()).or_default();
        match event {
            ToolBatchEvent::Started { call_id, .. } => {
                let id = ToolCallId(call_id.as_str().to_owned());
                if let Some(tool) = output.tools.get_mut(&id) {
                    self.access.observation.publish(ObservationEvent::Activity(
                        ActivityEvent::ToolStarted {
                            source: ToolCallIdentity {
                                turn: source.clone(),
                                tool_call_id: id.clone(),
                            },
                            data: ToolInvocationData {
                                name: tool.data.state.name.clone(),
                                arguments: tool.data.arguments.clone(),
                            },
                        },
                    ));
                    if tool.data.state.state == ToolCallState::Finished {
                        return;
                    }
                    tool.data.state.state = ToolCallState::Running;
                    tool.data.state.runner_revision = revision;
                    self.access.observation.publish(ObservationEvent::State(
                        StateEvent::Toolcall {
                            source: ToolCallIdentity {
                                turn: source,
                                tool_call_id: id,
                            },
                            data: tool.data.state.clone(),
                        },
                    ));
                    self.publish(&mut state);
                }
            }
            ToolBatchEvent::Progress { call_id, progress } => {
                let id = ToolCallId(call_id.as_str().to_owned());
                if let Some(tool) = output.tools.get_mut(&id)
                    && tool.data.state.state != ToolCallState::Finished
                {
                    tool.data.state.progress = Some(ToolProgressData {
                        message: progress.message.map(|v| v.as_str().to_owned()),
                        completed: progress.completed,
                        total: progress.total,
                    });
                    tool.data.state.runner_revision = revision;
                    self.access.observation.publish(ObservationEvent::State(
                        StateEvent::Toolcall {
                            source: ToolCallIdentity {
                                turn: source,
                                tool_call_id: id,
                            },
                            data: tool.data.state.clone(),
                        },
                    ));
                }
            }
            // The returned ToolResult, plus actual worker cleanup, determines
            // public finished. A progress event never settles the invocation.
            ToolBatchEvent::Finished { .. } => {}
        }
    }
    pub(super) fn observe_finished(
        &self,
        state: &mut SessionState,
        info: &TurnInfo,
        deadline_expired: bool,
    ) {
        // Finished is a new state transition, not another view of Finalizing.
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let revision = state.runner_revision;
        if let Some(resolution) = match info.outcome {
            Some(TurnOutcome::Cancelled) if deadline_expired => {
                Some(InteractionResolution::Expired)
            }
            Some(TurnOutcome::Cancelled) => Some(InteractionResolution::Cancelled),
            Some(TurnOutcome::Failed) => Some(InteractionResolution::Failed),
            _ => None,
        } && let Some(output) = state.output.get_mut(&info.id)
        {
            for record in output
                .interactions
                .values_mut()
                .filter(|record| !record.resolved)
            {
                record.resolved = true;
                self.access.observation.publish(ObservationEvent::Activity(
                    ActivityEvent::InteractionResolved {
                        source: ToolCallIdentity {
                            turn: self.turn_identity(info),
                            tool_call_id: record.data.tool_call_id.clone(),
                        },
                        interaction_id: record.data.interaction_id.clone(),
                        data: InteractionResolvedData { resolution },
                    },
                ));
            }
        }
        let source = self.turn_identity(info);
        let data = Self::turn_state(info, revision);
        state
            .output
            .entry(info.id.clone())
            .or_default()
            .terminal_state = Some(data.clone());
        self.access
            .observation
            .publish(ObservationEvent::State(StateEvent::Turn {
                source: source.clone(),
                data: data.clone(),
            }));
        self.access
            .observation
            .publish(ObservationEvent::Activity(ActivityEvent::TurnFinished {
                source,
                data,
            }));
    }
    pub(crate) fn observation_baseline(
        &self,
        turn: Option<&TurnId>,
        states: bool,
        outputs: bool,
    ) -> Result<(Vec<StateEvent>, Vec<OutputPosition>), Fault> {
        let state = self.state.lock().expect("Session mutex poisoned");
        if state.reclaimed {
            return Err(Fault::new(
                FaultKind::AgentSessionNotFound,
                "Session source was reclaimed",
            ));
        }
        let mut initial = Vec::new();
        let mut watermarks = Vec::new();
        if states && turn.is_none() {
            initial.push(StateEvent::Session {
                source: self.identity(),
                data: Self::make_snapshot(
                    &state,
                    &self.host_id,
                    &self.incarnation,
                    &self.agent_id,
                    &self.id,
                )
                .data,
            });
        }
        let turns = state
            .completed
            .values()
            .map(|result| &result.info)
            .chain(state.active.iter().map(|active| &active.info));
        let mut found = turn.is_none();
        for info in turns.filter(|info| turn.is_none_or(|id| id == &info.id)) {
            found = true;
            let source = self.turn_identity(info);
            if states {
                initial.push(StateEvent::Turn {
                    source: source.clone(),
                    data: Self::retained_turn_state(&state, info),
                });
            }
            if let Some(observation) = state.output.get(&info.id) {
                for (id, record) in &observation.tools {
                    let tool = ToolCallIdentity {
                        turn: source.clone(),
                        tool_call_id: id.clone(),
                    };
                    if states {
                        initial.push(StateEvent::Toolcall {
                            source: tool.clone(),
                            data: record.data.state.clone(),
                        });
                    }
                    if outputs {
                        for (channel, body) in &record.output {
                            watermarks.push(OutputPosition {
                                source: OutputSource::Toolcall {
                                    source: tool.clone(),
                                },
                                channel: channel.clone(),
                                end: body.text().len() as u64,
                            });
                        }
                    }
                }
                if outputs {
                    for (response_index, channels) in &observation.models {
                        for (channel, body) in channels {
                            watermarks.push(OutputPosition {
                                source: OutputSource::Model {
                                    source: ModelOutputIdentity {
                                        turn: source.clone(),
                                        response_index: *response_index,
                                    },
                                },
                                channel: channel.clone(),
                                end: body.text().len() as u64,
                            });
                        }
                    }
                }
            }
        }
        if !found {
            return Err(Fault::new(
                FaultKind::SessionTurnNotFound,
                "Turn is not held by this Session",
            ));
        }
        Ok((initial, watermarks))
    }
}
fn positions(channels: &BTreeMap<String, Body>) -> Vec<ChannelPosition> {
    channels
        .iter()
        .map(|(channel, body)| ChannelPosition {
            channel: channel.clone(),
            end: body.text().len() as u64,
        })
        .collect()
}

/// A connection-scoped state subscription. It never exposes a raw watch handle
/// that could keep receiving private state after that connection is revoked.
pub struct SessionSubscription {
    owner: Weak<SessionController>,
    snapshots: watch::Receiver<SessionSnapshot>,
    revocation: watch::Receiver<bool>,
    access: Arc<ConnectionRegistry>,
    caller: CallerContext,
}
impl SessionSubscription {
    pub fn snapshot(&self) -> Result<SessionSnapshot, Fault> {
        self.access.read(&self.caller, || {
            self.owner
                .upgrade()
                .ok_or_else(|| {
                    Fault::new(FaultKind::AgentSessionNotFound, "Session was reclaimed")
                })?
                .ensure_visible()?;
            Ok(self.snapshots.borrow().clone())
        })
    }
    pub async fn changed(&mut self) -> Result<(), Fault> {
        self.snapshot()?;
        tokio::select! {
            _ = self.revocation.changed() => Err(Fault::new(FaultKind::HostAccessInvalid, "subscription connection is no longer valid")),
            changed = self.snapshots.changed() => {
                changed.map_err(|_| Fault::new(FaultKind::SessionLifecycleClosed, "state subscription owner has ended"))?;
                self.snapshot().map(|_| ())
            }
        }
    }
}

impl SessionController {
    pub(super) fn make_snapshot(
        state: &SessionState,
        host_id: &HostId,
        incarnation: &HostIncarnation,
        agent_id: &AgentId,
        id: &SessionId,
    ) -> SessionSnapshot {
        let control = Self::control_facts(state);
        SessionSnapshot {
            host_id: host_id.clone(),
            host_incarnation: incarnation.clone(),
            agent_id: agent_id.clone(),
            session_id: id.clone(),
            data: SessionStateData {
                owned_work: Self::owned_work(state),
                end_reason: state.end_reason.clone(),
                mode: state.info.mode,
                controller_revision: state.controller_revision,
                lifecycle: state.lifecycle,
                scheduling: control.scheduling,
                pause_reasons: control.pause_reasons,
                residency: state.residency,
                load_id: state.load_id.clone(),
                runner_revision: state.runner_revision,
                active_turn_id: state.active.as_ref().map(|active| active.info.id.clone()),
                pending_interactions: state
                    .active
                    .as_ref()
                    .and_then(|active| {
                        active
                            .pending_interaction
                            .as_ref()
                            .map(|pending| pending.data.clone())
                    })
                    .into_iter()
                    .collect(),
                next_step: state.next_step.iter().map(QueuedInput::summary).collect(),
                next_turn: state.next_turn.iter().map(QueuedInput::summary).collect(),
                preparing: state.preparing,
                compaction: state
                    .compaction
                    .as_ref()
                    .map(|work| work.activity.clone())
                    .or_else(|| state.last_compaction.clone())
                    .map(Box::new),
                saving: state.history_work.as_ref().is_some_and(|work| work.running),
                deletion: state.deletion.clone(),
                metadata_saving: state.record_work.as_ref().is_some_and(|work| work.running),
                metadata_fault: state
                    .record_work
                    .as_ref()
                    .and_then(|work| work.fault.clone())
                    .or_else(|| state.close_fault.clone()),
                block_reason: control.block_reason,
                last_result: state.completion_order.last().map(|id| TurnRef {
                    session_id: state.info.id.clone(),
                    turn_id: id.clone(),
                }),
            },
        }
    }
    pub(super) fn publish(&self, state: &mut SessionState) {
        state.controller_revision = state
            .controller_revision
            .checked_add(1)
            .expect("controller revision exhausted");
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let snapshot = Self::make_snapshot(
            state,
            &self.host_id,
            &self.incarnation,
            &self.agent_id,
            &self.id,
        );
        self.publish_observation(state, &snapshot);
        self.snapshots.send_replace(snapshot);
        self.changed.notify_waiters();
        if let Some(parent_changed) = &self.parent_changed {
            parent_changed.notify_waiters();
        }
    }
    pub(crate) fn subscribe_state(
        self: &Arc<Self>,
        caller: &CallerContext,
    ) -> Result<SessionSubscription, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            Ok(SessionSubscription {
                owner: Arc::downgrade(self),
                snapshots: self.snapshots.subscribe(),
                revocation: caller.revocation(),
                access: Arc::clone(&self.access),
                caller: caller.clone(),
            })
        })
    }
}
