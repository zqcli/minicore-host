//! Scalar facts from the actual result-transfer boundary, never result bodies.
use super::*;
use crate::collaboration::DelegationRegistration;
use minicore_host_diagnostics::LogScalar;

pub(in crate::session) fn handoff_data(
    source: &SessionIdentity,
    registration: &DelegationRegistration,
    turns: Vec<LogScalar>,
    input_ranges: usize,
    discarded: usize,
) -> EventData {
    EventData::default()
        .context(Context {
            message_id: Some(registration.task_message_id.clone()),
            ..diagnostic::session_context(source)
        })
        .attribute("origin_agent_id", registration.parent.agent_id.as_str())
        .attribute("origin_session_id", registration.parent.session_id.as_str())
        .attribute("origin_turn_id", registration.origin.turn_id.as_str())
        .attribute(
            "origin_tool_call_id",
            registration.origin.tool_call_id.as_str(),
        )
        .attribute("turn_results", turns.len() as u64)
        .attribute("result_turn_ids", turns)
        .attribute("input_ranges", input_ranges as u64)
        .attribute("discarded_inputs", discarded as u64)
}
pub(in crate::session) fn handoff_finished(mut data: EventData, fault: Option<&Fault>) -> Record {
    let level = match fault {
        None => Level::INFO,
        Some(fault) => {
            data = data.fault(fault);
            if matches!(
                fault.kind(),
                Some(FaultKind::SessionInputInvalid | FaultKind::SessionBusy)
            ) {
                Level::WARN
            } else {
                Level::ERROR
            }
        }
    };
    Record::new(
        event(
            level,
            Scope::Session,
            "session.collaboration",
            if fault.is_some() {
                "message.result_handoff_failed"
            } else {
                "message.result_handed_off"
            },
            "Session received the actual result handoff disposition",
            &[
                "origin_agent_id",
                "origin_session_id",
                "origin_turn_id",
                "origin_tool_call_id",
                "turn_results",
                "result_turn_ids",
                "input_ranges",
                "discarded_inputs",
                "controller_revision",
            ],
        ),
        data,
    )
}
