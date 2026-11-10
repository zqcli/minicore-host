mod query;

use super::*;
use crate::diagnostics::{self as diagnostic, Record};
use crate::observation::*;
use minicore_host_diagnostics::{EventData, Scope};
use minicore_runtime::{LoopEvent, OutputChannel, model::AssistantPart};
use tracing::Level;

#[derive(Default)]
pub(super) struct TurnObservation {
    // Retained with the original Turn, never projected from a later Runner.
    terminal_state: Option<TurnStateData>,
    pub(super) interactions: BTreeMap<minicore_runtime::InteractionId, InteractionRecord>,
    pub(super) models: BTreeMap<u64, BTreeMap<String, Body>>,
    pub(super) tools: BTreeMap<ToolCallId, ToolRecord>,
}
pub(super) struct InteractionRecord {
    pub(super) data: PendingInteractionData,
    pub(super) resolved: bool,
    pub(super) answering: Option<watch::Receiver<bool>>,
}
impl TurnObservation {
    fn interaction(
        &mut self,
        pending: &minicore_runtime::interaction::PendingInteraction,
    ) -> &mut InteractionRecord {
        self.interactions
            .entry(pending.interaction_id)
            .or_insert_with(|| InteractionRecord {
                data: PendingInteractionData::from_runtime(
                    pending,
                    InteractionId::new().expect("Host interaction identity generation"),
                ),
                resolved: false,
                answering: None,
            })
    }
}
#[derive(Default)]
pub(super) struct Body {
    pub(super) live: String,
    // A binary channel has only a real contiguous UTF-8 prefix. Never imply
    // that a request crossing its end returned the complete process stream.
    pub(super) non_utf8: bool,
    complete: Option<String>,
}
impl Body {
    fn text(&self) -> &str {
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
        let base = active.response_base;
        let revision = state.runner_revision;
        let observation = state.output.entry(active.info.id.clone()).or_default();
        for item in items {
            match item {
                HistoryItem::Assistant(result) => {
                    let response_index = base + u64::from(result.request_index);
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

    pub(super) fn observe_runtime_state(
        &self,
        turn_id: &TurnId,
        loop_id: LoopId,
        runtime: minicore_runtime::LoopState,
    ) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.active.as_ref().is_none_or(|a| {
            &a.info.id != turn_id || a.loop_id != Some(loop_id) || runtime.loop_id != loop_id
        }) {
            return;
        }
        let pending = runtime.pending_interaction.as_ref().and_then(|pending| {
            let record = state
                .output
                .entry(turn_id.clone())
                .or_default()
                .interaction(pending);
            (!record.resolved).then(|| record.data.clone())
        });
        let active = state.active.as_mut().expect("checked original Turn");
        let response_index = active.response_base + u64::from(runtime.request_index);
        active.pending_interaction = pending;
        active.info.phase = match runtime.status {
            minicore_runtime::LoopStatus::RunningModel => Some(TurnPhase::Model),
            minicore_runtime::LoopStatus::RunningTools => Some(TurnPhase::Tools),
            minicore_runtime::LoopStatus::WaitingForInput => Some(TurnPhase::WaitingForInput),
            minicore_runtime::LoopStatus::Finishing => Some(TurnPhase::Finalizing),
            _ => None,
        };
        active.info.model_retry = runtime.model_retry.map(|retry| ModelRetryProgress {
            response_index,
            attempt: retry.attempt,
            max_attempts: retry.max_attempts,
            fault: runtime_model_fault(&retry.error),
            phase: match retry.phase {
                minicore_runtime::ModelRetryPhase::Waiting { retry_at, .. } => {
                    ModelRetryPhase::Waiting {
                        retry_at: retry_at.into(),
                    }
                }
                minicore_runtime::ModelRetryPhase::Requesting => ModelRetryPhase::Requesting,
            },
        });
        if runtime.model_attempts > 0 {
            let index = response_index as usize;
            active
                .info
                .model_attempts
                .resize(active.info.model_attempts.len().max(index + 1), 0);
            active.info.model_attempts[index] = runtime.model_attempts;
        }
        self.publish(&mut state);
    }
    pub(super) fn observe_runtime(&self, turn_id: &TurnId, loop_id: LoopId, event: LoopEvent) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let Some(active) = state
            .active
            .as_ref()
            .filter(|active| &active.info.id == turn_id && active.loop_id == Some(loop_id))
        else {
            return;
        };
        let source = self.turn_identity(&active.info);
        let base = active.response_base;
        // RequestStarted is an actual Runtime fact consumed by this direct
        // owner, not an observer callback or a reconstructed provider request.
        let diagnostic = match &event {
            LoopEvent::RequestStarted {
                loop_id: actual,
                request_index,
                ..
            } if *actual == loop_id => Some(Record::new(
                diagnostic::event(
                    Level::DEBUG,
                    Scope::Runtime,
                    "session.runtime",
                    "model.started",
                    "Runtime started a model request",
                    &["response_index", "loop_id", "request_index"],
                ),
                EventData::default()
                    .context(diagnostic::turn_context(&source))
                    .attribute("response_index", base + u64::from(*request_index))
                    .attribute("loop_id", actual.to_string())
                    .attribute("request_index", u64::from(*request_index)),
            )),
            _ => None,
        };
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let revision = state.runner_revision;
        let output = state.output.entry(turn_id.clone()).or_default();
        match event {
            LoopEvent::RequestStarted {
                loop_id: actual,
                request_index,
                ..
            } if actual == loop_id => {
                let channels = output
                    .models
                    .entry(base + u64::from(request_index))
                    .or_default();
                channels.entry("text".into()).or_default();
                channels.entry("reasoning".into()).or_default();
            }
            LoopEvent::OutputDelta {
                loop_id: actual,
                request_index,
                channel,
                delta,
            } if actual == loop_id => {
                let response_index = base + u64::from(request_index);
                let channel = match channel {
                    OutputChannel::Text => "text",
                    OutputChannel::Reasoning => "reasoning",
                };
                let body = output
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
            LoopEvent::ToolStarted {
                loop_id: actual,
                call_id,
                ..
            } if actual == loop_id => {
                if let Some(tool) = output
                    .tools
                    .get_mut(&ToolCallId(call_id.as_str().to_owned()))
                {
                    self.access.observation.publish(ObservationEvent::Activity(
                        ActivityEvent::ToolStarted {
                            source: ToolCallIdentity {
                                turn: source.clone(),
                                tool_call_id: ToolCallId(call_id.as_str().to_owned()),
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
                                tool_call_id: ToolCallId(call_id.as_str().to_owned()),
                            },
                            data: tool.data.state.clone(),
                        },
                    ));
                    self.publish(&mut state);
                }
            }
            LoopEvent::ToolProgress {
                loop_id: actual,
                call_id,
                progress,
                ..
            } if actual == loop_id => {
                if let Some(tool) = output
                    .tools
                    .get_mut(&ToolCallId(call_id.as_str().to_owned()))
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
                                tool_call_id: ToolCallId(call_id.as_str().to_owned()),
                            },
                            data: tool.data.state.clone(),
                        },
                    ));
                }
            }
            LoopEvent::InteractionRequested {
                loop_id: actual,
                interaction,
            } if actual == loop_id => {
                let pending = output.interaction(&interaction).data.clone();
                self.access.observation.publish(ObservationEvent::Activity(
                    ActivityEvent::InteractionRequested {
                        source: ToolCallIdentity {
                            turn: source,
                            tool_call_id: pending.tool_call_id,
                        },
                        interaction_id: pending.interaction_id,
                        data: pending.request,
                    },
                ));
            }
            LoopEvent::InteractionResolved {
                loop_id: actual,
                interaction_id,
            } if actual == loop_id => {
                if let Some(record) = output.interactions.get_mut(&interaction_id)
                    && !record.resolved
                {
                    record.resolved = true;
                    let tool_call_id = record.data.tool_call_id.clone();
                    let interaction_id = record.data.interaction_id.clone();
                    self.access.observation.publish(ObservationEvent::Activity(
                        ActivityEvent::InteractionResolved {
                            source: ToolCallIdentity {
                                turn: source,
                                tool_call_id,
                            },
                            interaction_id,
                            data: InteractionResolvedData {
                                resolution: InteractionResolution::Answered,
                            },
                        },
                    ));
                }
            }
            // ToolFinished is deliberately not used to synthesize a ToolResult.
            _ => {}
        }
        drop(state);
        // Called only by the owned Runtime receiver outside admission scopes.
        // Both Session state and observation publication locks have ended.
        diagnostic::emit(diagnostic);
    }
    pub(super) fn observe_finished(
        &self,
        state: &mut SessionState,
        info: &TurnInfo,
        report: Option<&LoopReport>,
    ) {
        // Finished is a new state transition, not another view of Finalizing.
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let revision = state.runner_revision;
        if let Some(resolution) = match info.outcome {
            Some(TurnOutcome::Cancelled)
                if report.is_some_and(|report| {
                    report.outcome
                        == LoopOutcome::Cancelled(minicore_runtime::CancelReason::Deadline)
                }) =>
            {
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
    pub(crate) fn output_read(
        &self,
        caller: &CallerContext,
        turn: &TurnId,
        response: Option<u64>,
        tool: Option<&ToolCallId>,
        request: &TextReadRequest,
    ) -> Result<TextRangeData, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            request
                .validate()
                .map_err(|e| Fault::new(FaultKind::SessionOutputRangeInvalid, e.to_string()))?;
            let state = self.state.lock().expect("Session mutex poisoned");
            let output = state.output.get(turn).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionTurnNotFound,
                    "Turn output is not retained",
                )
            })?;
            let channels = match (response, tool) {
                (Some(index), None) => output.models.get(&index),
                (None, Some(id)) => output.tools.get(id).map(|record| &record.output),
                _ => None,
            }
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionOutputNotFound,
                    "output source is not retained",
                )
            })?;
            let body = channels.get(&request.channel).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionOutputNotFound,
                    "output channel is not retained",
                )
            })?;
            if body.non_utf8 && request.end > body.text().len() as u64 {
                return Err(Fault::new(
                    FaultKind::SessionOutputRangeInvalid,
                    "tool channel contains non-UTF-8 bytes; only its observed UTF-8 prefix is text-readable",
                ));
            }
            read_range(body.text(), request)
        })
    }
    pub(crate) fn tool_get(
        &self,
        caller: &CallerContext,
        turn: &TurnId,
        tool: &ToolCallId,
    ) -> Result<(ToolCallIdentity, ToolCallData), Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            let info = state
                .active
                .as_ref()
                .filter(|a| &a.info.id == turn)
                .map(|a| &a.info)
                .or_else(|| state.completed.get(turn).map(|r| &r.info))
                .ok_or_else(|| {
                    Fault::new(FaultKind::SessionTurnNotFound, "Turn is not retained")
                })?;
            let record = state
                .output
                .get(turn)
                .and_then(|o| o.tools.get(tool))
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionToolCallNotFound,
                        "ToolCall is not retained by this Turn",
                    )
                })?;
            Ok((
                ToolCallIdentity {
                    turn: self.turn_identity(info),
                    tool_call_id: tool.clone(),
                },
                record.data.clone(),
            ))
        })
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
fn read_range(text: &str, request: &TextReadRequest) -> Result<TextRangeData, Fault> {
    let invalid = || {
        Fault::new(
            FaultKind::SessionOutputRangeInvalid,
            "read positions must be available UTF-8 boundaries",
        )
    };
    let start = usize::try_from(request.start).map_err(|_| invalid())?;
    let requested_end = usize::try_from(request.end).map_err(|_| invalid())?;
    if start > text.len()
        || !text.is_char_boundary(start)
        || (requested_end <= text.len() && !text.is_char_boundary(requested_end))
    {
        return Err(invalid());
    }
    let fixed_end = requested_end.min(text.len());
    let budget = usize::try_from(request.max_bytes).unwrap_or(usize::MAX);
    let mut end = start.saturating_add(budget).min(fixed_end);
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start && start < fixed_end {
        end += text[start..fixed_end]
            .chars()
            .next()
            .expect("nonempty range")
            .len_utf8();
    }
    Ok(TextRangeData {
        channel: request.channel.clone(),
        start: start as u64,
        end: end as u64,
        text: text[start..end].to_owned(),
        has_more: end < fixed_end,
    })
}
