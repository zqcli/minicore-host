//! Independent faults returned by the real cleanup owner, not Runtime outcome.
use super::*;

pub(in crate::session) fn cleanup_failed(
    source: &TurnIdentity,
    info: &TurnInfo,
    fault: &Fault,
    confirmed: bool,
) -> Record {
    Record::new(
        event(
            Level::ERROR,
            Scope::Turn,
            "session.tool_cleanup",
            "turn.cleanup_failed",
            "Turn cleanup owner reported an independent failure",
            &[
                "phase",
                "outcome",
                "persistence",
                "requests",
                "tool_rounds",
                "controller_revision",
                "runner_revision",
                "cleanup_confirmed",
            ],
        ),
        turn_data(source, info)
            .fault(fault)
            .attribute("phase", "finalizing")
            .attribute("cleanup_confirmed", confirmed),
    )
}
