//! Small owner-side metadata extraction. Records contain no owner or payload.
//! Callers prepare facts under their existing boundary and emit after all locks.
use minicore_host_diagnostics::{Context, EventData, EventSpec, Scope};
use minicore_host_protocol::{SessionIdentity, TurnIdentity};
use tracing::Level;

#[must_use = "emit metadata after the owning lock scope has ended"]
pub(crate) struct Record(EventSpec, EventData);
impl Record {
    pub(crate) fn new(spec: EventSpec, data: EventData) -> Self {
        Self(spec, data)
    }
    /// Attach the revisions actually published by the same owner boundary.
    /// Preparing a public observation may precede publish; emission never does.
    pub(crate) fn published(mut self, controller_revision: u64, runner_revision: u64) -> Self {
        if self.0.attributes.contains(&"controller_revision") {
            self.1 = self.1.attribute("controller_revision", controller_revision);
        }
        if self.0.attributes.contains(&"runner_revision") {
            self.1 = self.1.attribute("runner_revision", runner_revision);
        }
        self
    }
    pub(crate) fn emit(self) {
        self.0.emit(|| self.1);
    }
}
pub(crate) fn emit(records: impl IntoIterator<Item = Record>) {
    for record in records {
        record.emit();
    }
}
pub(crate) const fn event(
    level: Level,
    scope: Scope,
    component: &'static str,
    event: &'static str,
    message: &'static str,
    attributes: &'static [&'static str],
) -> EventSpec {
    EventSpec {
        level,
        scope,
        component,
        event,
        message,
        attributes,
    }
}
pub(crate) fn session_context(source: &SessionIdentity) -> Context {
    Context {
        host_id: Some(source.host_id.clone()),
        host_incarnation: Some(source.host_incarnation.clone()),
        agent_id: Some(source.agent_id.clone()),
        session_id: Some(source.session_id.clone()),
        ..Context::default()
    }
}
pub(crate) fn turn_context(source: &TurnIdentity) -> Context {
    Context {
        turn_id: Some(source.turn_id.clone()),
        load_id: source.load_id.clone(),
        ..session_context(&source.session)
    }
}

/// A failed internal invocation is not a second primary business ERROR. The
/// caller supplies its original source and request identity leaves only.
pub(crate) fn message_error(
    context: Context,
    target: &minicore_host_protocol::SessionRef,
    task: &minicore_host_protocol::MessageId,
    origin: Option<&minicore_host_protocol::ToolCallRef>,
    phase: &'static str,
    fault: &minicore_host_protocol::Fault,
) -> Record {
    use minicore_host_protocol::FaultKind;
    let scope = if context.turn_id.is_some() {
        Scope::Turn
    } else {
        Scope::Session
    };
    let mut data = EventData::default()
        .context(context)
        .fault(fault)
        .attribute("phase", phase)
        .attribute("target_agent_id", target.agent_id.as_str())
        .attribute("target_session_id", target.session_id.as_str())
        .attribute("task_message_id", task.as_str());
    if let Some(origin) = origin {
        data = data
            .attribute("origin_turn_id", origin.turn_id.as_str())
            .attribute("origin_tool_call_id", origin.tool_call_id.as_str());
    }
    Record::new(
        event(
            if matches!(
                fault.kind(),
                Some(
                    FaultKind::SessionInputInvalid
                        | FaultKind::SessionBusy
                        | FaultKind::SessionOwnershipMismatch
                )
            ) {
                Level::WARN
            } else {
                Level::DEBUG
            },
            scope,
            "message.internal",
            "message.send_error",
            "Original internal message invocation returned an error",
            &[
                "phase",
                "target_agent_id",
                "target_session_id",
                "task_message_id",
                "origin_turn_id",
                "origin_tool_call_id",
            ],
        ),
        data,
    )
}
