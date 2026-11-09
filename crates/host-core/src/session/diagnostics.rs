//! Scalar extraction for real Session and Turn fact boundaries.
use super::*;
use crate::diagnostics::{event, turn_context};
use minicore_host_diagnostics::{Context, EventData, Scope};
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
