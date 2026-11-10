//! Scalar extraction for real Session and Turn fact boundaries.
mod runtime;
use super::*;
use crate::diagnostics::{event, turn_context};
use minicore_host_diagnostics::{Context, EventData, Scope};
pub(super) use runtime::{model_retry, models_reported};
use tracing::Level;

const INPUT_ATTRIBUTES: &[&str] = &[
    "queue",
    "order",
    "mode",
    "controller_revision",
    "next_step_len",
    "next_turn_len",
    "origin_agent_id",
    "origin_session_id",
];
pub(crate) fn input(
    context: Context,
    record: &MessageRecord,
    revision: u64,
    next_step: usize,
    next_turn: usize,
    bound: bool,
) -> Record {
    let spec = event(
        Level::INFO,
        Scope::Session,
        "session.controller",
        if bound {
            "session.input_bound"
        } else {
            "session.input_accepted"
        },
        if bound {
            "Input bound to its actual Turn"
        } else {
            "Input accepted into the actual Session queue"
        },
        INPUT_ATTRIBUTES,
    );
    let mut data = EventData::default()
        .context(Context {
            message_id: Some(record.input.message_id.clone()),
            in_reply_to: record.input.in_reply_to.clone(),
            ..context
        })
        .attribute(
            "queue",
            match record.queue {
                InputQueue::NextStep => "next-step",
                InputQueue::NextTurn => "next-turn",
            },
        )
        .attribute(
            "mode",
            match record.input.mode {
                MessageMode::FollowUp => "follow_up",
                MessageMode::Steer => "steer",
            },
        )
        .attribute("order", record.order)
        .attribute("controller_revision", revision)
        .attribute("next_step_len", next_step as u64)
        .attribute("next_turn_len", next_turn as u64);
    if let MessageSource::Agent { source } = &record.source {
        data = data
            .attribute("origin_agent_id", source.agent_id.as_str())
            .attribute("origin_session_id", source.session_id.as_str());
    }
    Record::new(spec, data)
}

pub(super) fn turn_started(source: &TurnIdentity, info: &TurnInfo, revision: u64) -> Record {
    Record::new(
        event(
            Level::INFO,
            Scope::Turn,
            "turn.owner",
            "turn.started",
            "Turn bound to its selected execution resources",
            &["phase", "controller_revision", "config_revision"],
        ),
        EventData::default()
            .context(turn_context(source))
            .attribute("phase", "starting")
            .attribute("controller_revision", revision)
            .attribute("config_revision", info.config_revision),
    )
}

pub(crate) fn history_failed(
    context: Context,
    fault: &Fault,
    phase: &'static str,
    revision: u64,
) -> Record {
    let load = fault.kind() == Some(FaultKind::SessionHistoryLoadFailed);
    Record::new(
        event(
            Level::ERROR,
            Scope::Session,
            "session.history",
            if load {
                "session.history_read_failed"
            } else {
                "session.history_write_failed"
            },
            "Session history operation failed; original business fault is retained",
            &["phase", "controller_revision"],
        ),
        EventData::default()
            .context(context)
            .fault(fault)
            .attribute("phase", phase)
            .attribute("controller_revision", revision),
    )
}

fn persistence(value: PersistenceState) -> &'static str {
    match value {
        PersistenceState::NotApplicable => "not_applicable",
        PersistenceState::Pending => "pending",
        PersistenceState::Persisted => "persisted",
        PersistenceState::Failed => "failed",
    }
}
const TURN_ATTRIBUTES: &[&str] = &[
    "phase",
    "outcome",
    "persistence",
    "controller_revision",
    "runner_revision",
    "duration_ms",
    "requests",
    "tool_rounds",
];
fn turn_data(source: &TurnIdentity, info: &TurnInfo) -> EventData {
    let mut data = EventData::default()
        .context(turn_context(source))
        .attribute("persistence", persistence(info.persistence))
        .attribute("requests", info.requests)
        .attribute("tool_rounds", info.tool_rounds);
    if let Some(outcome) = info.outcome {
        data = data.attribute(
            "outcome",
            match outcome {
                TurnOutcome::Completed => "completed",
                TurnOutcome::Cancelled => "cancelled",
                TurnOutcome::Failed => "failed",
            },
        );
    }
    if let Some(fault) = &info.fault {
        data = data.fault(fault);
    }
    data
}
pub(super) fn turn_finished(source: &TurnIdentity, info: &TurnInfo, start: Instant) -> Record {
    let mut data = turn_data(source, info).attribute("phase", "finished");
    if let Ok(duration) = u64::try_from(start.elapsed().as_millis()) {
        data = data.attribute("duration_ms", duration);
    }
    Record::new(
        event(
            Level::INFO,
            Scope::Turn,
            "turn.owner",
            "turn.finished",
            "Turn finished after its original save and cleanup barriers",
            TURN_ATTRIBUTES,
        ),
        data,
    )
}
impl SessionController {
    /// First claim is an existing outcome transition, not a logging flag.
    /// History-caused failures already have their direct storage ERROR.
    pub(super) fn terminal_claim_record(
        &self,
        state: &SessionState,
        newly_claimed: bool,
    ) -> Option<Record> {
        if !newly_claimed {
            return None;
        }
        let active = state.active.as_ref()?;
        if active.info.outcome != Some(TurnOutcome::Failed) {
            return None;
        }
        active.info.fault.as_ref()?;
        Some(Record::new(
            event(
                Level::ERROR,
                Scope::Turn,
                "turn.owner",
                "turn.failed",
                "Turn owner claimed its primary execution failure",
                TURN_ATTRIBUTES,
            ),
            turn_data(&self.turn_identity(&active.info), &active.info)
                .attribute("phase", "finalizing")
                .attribute("controller_revision", state.controller_revision)
                .attribute("runner_revision", state.runner_revision),
        ))
    }
}

impl SessionController {
    pub(super) fn cancellable(state: &SessionState) -> bool {
        state.active.as_ref().is_some_and(|active| {
            matches!(active.info.state, TurnState::Starting | TurnState::Running,)
        })
    }
    pub(super) fn cancel_claim_record(
        &self,
        state: &SessionState,
        claimed: bool,
        reason: &'static str,
        operation: Option<OperationId>,
        published: bool,
    ) -> Option<Record> {
        if !claimed {
            return None;
        }
        let active = state.active.as_ref()?;
        let mut data = EventData::default()
            .context(Context {
                operation_id: operation,
                ..turn_context(&self.turn_identity(&active.info))
            })
            .attribute("phase", "cancelling")
            .attribute("reason", reason)
            .attribute("persistence", persistence(active.info.persistence));
        if published {
            data = data
                .attribute("controller_revision", state.controller_revision)
                .attribute("runner_revision", state.runner_revision);
        }
        Some(Record::new(
            event(
                Level::INFO,
                Scope::Turn,
                "turn.owner",
                "turn.cancel_claimed",
                "Turn owner claimed cancellation",
                &[
                    "phase",
                    "reason",
                    "persistence",
                    "controller_revision",
                    "runner_revision",
                ],
            ),
            data,
        ))
    }
    pub(super) fn closed_record(
        &self,
        state: &SessionState,
        previous: SessionLifecycle,
    ) -> Option<Record> {
        if !state.published
            || previous == SessionLifecycle::Closed
            || state.lifecycle != SessionLifecycle::Closed
        {
            return None;
        }
        Some(Record::new(
            event(
                Level::INFO,
                Scope::Session,
                "session.controller",
                "session.closed",
                "Session completed its actual close barriers",
                &["mode", "controller_revision"],
            ),
            EventData::default()
                .context(diagnostic::session_context(&self.identity()))
                .attribute("controller_revision", state.controller_revision)
                .attribute(
                    "mode",
                    match state.info.mode {
                        StorageMode::Memory => "memory",
                        StorageMode::Persistent => "persistent",
                    },
                ),
        ))
    }
    pub(super) fn close_records(
        &self,
        state: &SessionState,
        previous: SessionLifecycle,
        was_cancellable: bool,
        reason: &'static str,
        operation: Option<OperationId>,
    ) -> Vec<Record> {
        let mut records = Vec::new();
        if previous == SessionLifecycle::Open && state.lifecycle != SessionLifecycle::Open {
            records.push(Record::new(
                event(
                    Level::INFO,
                    Scope::Session,
                    "session.controller",
                    "session.close_accepted",
                    "Session owner accepted close intent",
                    &["reason", "controller_revision"],
                ),
                EventData::default()
                    .context(Context {
                        operation_id: operation.clone(),
                        ..diagnostic::session_context(&self.identity())
                    })
                    .attribute("reason", reason)
                    .attribute("controller_revision", state.controller_revision),
            ));
        }
        let claimed = was_cancellable
            && state
                .active
                .as_ref()
                .is_some_and(|active| active.info.state == TurnState::Cancelling);
        records.extend(self.cancel_claim_record(state, claimed, reason, operation, true));
        records.extend(self.closed_record(state, previous));
        records
    }
}

fn tool_context(source: &ToolCallIdentity) -> Context {
    Context {
        tool_call_id: Some(source.tool_call_id.clone()),
        ..turn_context(&source.turn)
    }
}
pub(super) fn tool_finished(
    source: &ToolCallIdentity,
    state: &crate::observation::ToolCallStateData,
    outcome: minicore_runtime::tools::ToolResultOutcome,
) -> Record {
    use minicore_runtime::tools::ToolResultOutcome;
    Record::new(
        event(
            Level::DEBUG,
            Scope::Runtime,
            "session.tools",
            "tool.finished",
            "Original tool result and its actual owned cleanup are complete",
            &["tool_name", "response_index", "runner_revision", "outcome"],
        ),
        EventData::default()
            .context(tool_context(source))
            .attribute("tool_name", state.name.as_str())
            .attribute("response_index", state.response_index)
            .attribute("runner_revision", state.runner_revision)
            .attribute(
                "outcome",
                match outcome {
                    ToolResultOutcome::Success => "success",
                    ToolResultOutcome::Failed => "failed",
                    ToolResultOutcome::Denied => "denied",
                    ToolResultOutcome::Cancelled => "cancelled",
                    ToolResultOutcome::InputProvided => "input_provided",
                },
            ),
    )
}
pub(super) fn process_exit(
    source: &ToolCallIdentity,
    name: &str,
    pid: u32,
    exit_code: Option<i32>,
    signal: Option<i32>,
    process_waited: bool,
    cleanup_confirmed: bool,
) -> Record {
    let mut data = EventData::default()
        .context(tool_context(source))
        .attribute("tool_name", name)
        .attribute("pid", u64::from(pid))
        .attribute("process_waited", process_waited)
        .attribute("cleanup_confirmed", cleanup_confirmed);
    if let Some(code) = exit_code {
        data = data.attribute("exit_code", i64::from(code));
    }
    if let Some(signal) = signal {
        data = data.attribute("signal", i64::from(signal));
    }
    Record::new(
        event(
            Level::DEBUG,
            Scope::Runtime,
            "session.tools",
            "tool.process_observed",
            "Command owner observed original process wait and cleanup facts",
            &[
                "tool_name",
                "pid",
                "exit_code",
                "signal",
                "process_waited",
                "cleanup_confirmed",
            ],
        ),
        data,
    )
}

pub(super) fn metadata_failed(source: &SessionIdentity, fault: &Fault, revision: u64) -> Record {
    Record::new(
        event(
            Level::ERROR,
            Scope::Session,
            "session.metadata",
            "session.metadata_save_failed",
            "Session metadata save failed; its original candidate remains owned",
            &["phase", "controller_revision"],
        ),
        EventData::default()
            .context(diagnostic::session_context(source))
            .fault(fault)
            .attribute("phase", "metadata_save")
            .attribute("controller_revision", revision),
    )
}
