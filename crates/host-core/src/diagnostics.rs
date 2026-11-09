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
