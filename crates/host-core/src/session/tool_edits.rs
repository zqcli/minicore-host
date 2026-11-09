//! Retained edit observations belong to the same Session as their actual tool.
use super::*;
use minicore_host_workspace::{
    ChangeRef, ChangesListRequest, ChangesListResult, EditOutcome, EditRecord, FileSnapshot,
    WorkspaceError,
};

impl SessionController {
    pub(super) fn record_tool_edit(
        &self,
        lease: &ToolOutputLease,
        path: String,
        before: FileSnapshot,
        after: FileSnapshot,
        outcome: EditOutcome,
    ) -> Result<(), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if !lease.admitted(&state) {
            return Err(Fault::new(
                FaultKind::TurnInternalUnexpected,
                "edit observation no longer belongs to its live tool cleanup owner",
            ));
        }
        state
            .edits
            .append(EditRecord {
                tool_call: ToolCallRef {
                    turn_id: lease.source().turn.turn_id.clone(),
                    tool_call_id: lease.source().tool_call_id.clone(),
                },
                path,
                before,
                after,
                outcome,
            })
            .map(|_| ())
            .map_err(|error| Fault::new(FaultKind::TurnInternalUnexpected, error.to_string()))
    }

    pub(crate) fn changes_list(
        &self,
        caller: &CallerContext,
        request: ChangesListRequest,
    ) -> Result<ChangesListResult, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            retained(&state)?;
            if request.scope == minicore_host_workspace::ChangeScope::Session && !state.edits_complete {
                return Err(Fault::new(FaultKind::SessionChangeSourceUnavailable,
                    "prior Session edit observations were not retained across restart; current retained Turns can be queried separately"));
            }
            if let minicore_host_workspace::ChangeScope::Turn { turn_id } = &request.scope
                && !state.output.contains_key(turn_id)
            {
                return Err(Fault::new(
                    FaultKind::SessionTurnNotFound,
                    "Turn edit observations are not retained",
                ));
            }
            state.edits.list(request).map_err(query_fault)
        })
    }

    pub(crate) fn change_record(
        &self,
        caller: &CallerContext,
        reference: &ChangeRef,
    ) -> Result<Arc<EditRecord>, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            retained(&state)?;
            state.edits.get(reference).map_err(query_fault)
        })
    }
}

fn query_fault(error: WorkspaceError) -> Fault {
    let kind = match error {
        WorkspaceError::ChangeNotFound => FaultKind::SessionChangeNotFound,
        WorkspaceError::InvalidCursor => FaultKind::SessionWorkspaceCursorInvalid,
        WorkspaceError::InvalidRange => FaultKind::SessionWorkspaceRangeInvalid,
        _ => FaultKind::SessionInternalUnexpected,
    };
    Fault::new(kind, error.to_string())
}

fn retained(state: &SessionState) -> Result<(), Fault> {
    if state.reclaimed {
        return Err(Fault::new(
            FaultKind::AgentSessionNotFound,
            "Session source was reclaimed",
        ));
    }
    Ok(())
}
