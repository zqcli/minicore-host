//! Original compaction identities and returned scalar facts, never its source,
//! candidate, prompt, summary, or provider error text.
use super::*;
use crate::session::compaction::CompactionBinding;

const ATTRIBUTES: &[&str] = &[
    "source",
    "phase",
    "receipt_index",
    "config_revision",
    "loop_id",
    "request_index",
    "response_index",
    "controller_revision",
    "cancellation_requested",
    "cancel_operation_id",
    "outcome",
    "persistence",
    "source_position",
    "covered_items",
    "before_tokens",
    "after_tokens",
    "duration_ms",
];

pub(in crate::session) fn compaction_context(
    source: &SessionIdentity,
    binding: &CompactionBinding,
) -> Context {
    let mut context = diagnostic::session_context(source);
    match binding {
        CompactionBinding::Manual { operation_id, .. } => {
            context.operation_id = Some(operation_id.clone());
        }
        CompactionBinding::Automatic {
            turn_id, load_id, ..
        } => {
            context.turn_id = Some(turn_id.clone());
            context.load_id = Some(load_id.clone());
        }
    }
    context
}

fn data(
    source: &SessionIdentity,
    binding: &CompactionBinding,
    activity: &CompactionActivity,
) -> EventData {
    let mut data = EventData::default()
        .context(compaction_context(source, binding))
        .attribute("cancellation_requested", activity.cancellation_requested);
    match binding {
        CompactionBinding::Manual {
            receipt_index,
            config_revision,
            ..
        } => {
            data = data
                .attribute("source", "manual")
                .attribute("receipt_index", *receipt_index as u64)
                .attribute("config_revision", *config_revision);
        }
        CompactionBinding::Automatic {
            loop_id,
            request_index,
            ..
        } => {
            data = data
                .attribute("source", "automatic")
                .attribute("loop_id", loop_id.to_string())
                .attribute("request_index", u64::from(*request_index));
            if let CompactionSource::Automatic { response_index, .. } = &activity.source {
                data = data.attribute("response_index", *response_index);
            }
        }
    }
    data = data.attribute(
        "phase",
        match &activity.state {
            CompactionState::Preparing {} => "preparing",
            CompactionState::Summarizing {} => "summarizing",
            CompactionState::Merging {} => "merging",
            CompactionState::Committing { .. } => "committing",
            CompactionState::Finished { .. } => "finished",
        },
    );
    let change = match &activity.state {
        CompactionState::Committing {
            change,
            persistence: saved,
            fault,
        } => {
            data = data.attribute("persistence", persistence(*saved));
            if let Some(fault) = fault {
                data = data.fault(fault);
            }
            Some(change)
        }
        CompactionState::Finished { result } => {
            data = data.attribute(
                "outcome",
                match result {
                    CompactionOutcome::Compacted { .. } => "compacted",
                    CompactionOutcome::Unchanged => "unchanged",
                    CompactionOutcome::Cancelled => "cancelled",
                    CompactionOutcome::Failed { .. } => "failed",
                },
            );
            match result {
                CompactionOutcome::Compacted {
                    change,
                    persistence: saved,
                } => {
                    data = data.attribute("persistence", persistence(*saved));
                    Some(change)
                }
                CompactionOutcome::Failed { fault } => {
                    data = data.fault(fault);
                    None
                }
                _ => None,
            }
        }
        _ => None,
    };
    if let Some(change) = change {
        data = data
            .attribute("source_position", change.source_position)
            .attribute("covered_items", change.covered_items)
            .attribute("before_tokens", change.before_tokens)
            .attribute("after_tokens", change.after_tokens);
    }
    data
}

pub(in crate::session) fn compaction_started(
    source: &SessionIdentity,
    binding: &CompactionBinding,
    activity: &CompactionActivity,
) -> Record {
    Record::new(
        event(
            Level::INFO,
            Scope::Session,
            "session.compaction",
            "session.compaction_started",
            "Session accepted its original compaction work",
            ATTRIBUTES,
        ),
        data(source, binding, activity),
    )
}
pub(in crate::session) fn compaction_cancelled(
    source: &SessionIdentity,
    binding: &CompactionBinding,
    activity: &CompactionActivity,
    cancel_operation: Option<&OperationId>,
) -> Record {
    let mut data = data(source, binding, activity);
    if let Some(operation) = cancel_operation {
        data = data.attribute("cancel_operation_id", operation.as_str());
    }
    Record::new(
        event(
            Level::INFO,
            Scope::Session,
            "session.compaction",
            "session.compaction_cancel_claimed",
            "Session claimed cancellation of its original compaction work",
            ATTRIBUTES,
        ),
        data,
    )
}
pub(in crate::session) fn compaction_committing(
    source: &SessionIdentity,
    binding: &CompactionBinding,
    activity: &CompactionActivity,
) -> Record {
    Record::new(
        event(
            Level::INFO,
            Scope::Session,
            "session.compaction",
            "session.compaction_committing",
            "Compaction candidate claimed its original save; confirmation is pending",
            ATTRIBUTES,
        ),
        data(source, binding, activity),
    )
}
pub(in crate::session) fn compaction_finished(
    source: &SessionIdentity,
    binding: &CompactionBinding,
    activity: &CompactionActivity,
    started: Instant,
) -> Record {
    let failed = matches!(
        activity.state,
        CompactionState::Finished {
            result: CompactionOutcome::Failed { .. }
        }
    );
    let mut data = data(source, binding, activity);
    if let Ok(duration) = u64::try_from(started.elapsed().as_millis()) {
        data = data.attribute("duration_ms", duration);
    }
    Record::new(
        event(
            if failed { Level::ERROR } else { Level::INFO },
            Scope::Session,
            "session.compaction",
            "session.compaction_finished",
            "Session settled the actual compaction outcome and applicable save confirmation",
            ATTRIBUTES,
        ),
        data,
    )
}

/// Utility request indices belong to the actual utility model calls; they are
/// not public response indices or fabricated dialogue Turns. Absent usage is
/// omitted rather than reconstructed from text or reported as zero.
pub(in crate::session) fn compaction_utilities(
    source: &SessionIdentity,
    binding: &CompactionBinding,
    activity: &CompactionActivity,
    loop_id: LoopId,
) -> Vec<Record> {
    activity
        .utility_usage
        .iter()
        .map(|request| {
            let mut data = data(source, binding, activity)
                .attribute("loop_id", loop_id.to_string())
                .attribute("utility_request_index", u64::from(request.request_index))
                .attribute("completed", request.completed);
            if let Some(usage) = request.usage {
                for (name, value) in [
                    ("input_tokens", usage.input_tokens()),
                    ("output_tokens", usage.output_tokens()),
                    ("reasoning_tokens", usage.reasoning_tokens()),
                    ("cache_read_tokens", usage.cache_read_tokens()),
                    ("cache_write_tokens", usage.cache_write_tokens()),
                    ("provider_total_tokens", usage.provider_total_tokens()),
                ] {
                    if let Some(value) = value {
                        data = data.attribute(name, value);
                    }
                }
            }
            Record::new(
                event(
                    Level::DEBUG,
                    Scope::Session,
                    "session.compaction",
                    "session.compaction_utility_finished",
                    "Compaction utility returned its actual request and usage facts",
                    &[
                        "source",
                        "receipt_index",
                        "config_revision",
                        "loop_id",
                        "request_index",
                        "response_index",
                        "utility_request_index",
                        "completed",
                        "input_tokens",
                        "output_tokens",
                        "reasoning_tokens",
                        "cache_read_tokens",
                        "cache_write_tokens",
                        "provider_total_tokens",
                        "controller_revision",
                    ],
                ),
                data,
            )
        })
        .collect()
}
