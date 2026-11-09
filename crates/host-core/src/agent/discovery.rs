use super::*;
use crate::{AgentSummary, SessionListPage, SessionListRequest};

impl AgentOwner {
    pub(crate) fn summary(&self, caller: &CallerContext) -> Result<AgentSummary, Fault> {
        let mut summary = self.access.read(caller, || {
            let state = self.state.lock().expect("Agent mutex poisoned");
            let info = &state.info;
            Ok(AgentSummary {
                id: info.id.clone(),
                host_id: info.host_id.clone(),
                name: info.name.clone(),
                mode: info.mode,
                lifecycle: info.lifecycle,
                config_revision: info.config_revision,
                default_session_id: info.default_session_id.clone(),
                metadata_saving: info.metadata_saving,
                metadata_fault: info.metadata_fault.clone(),
                deletion: info.deletion.as_ref().map(|deletion| deletion.data.clone()),
                sessions: Vec::new(),
            })
        })?;
        summary.sessions = self
            .session_page(caller, &SessionListRequest::default())?
            .items;
        Ok(summary)
    }

    pub(crate) fn session_page(
        &self,
        caller: &CallerContext,
        request: &SessionListRequest,
    ) -> Result<SessionListPage, Fault> {
        crate::discovery::validate_list_limit(request.limit)?;
        // Snapshot direct children, then query each only after releasing the
        // Agent lock. Selection is from summaries, never cloned full configs.
        let children = self.list_sessions(caller)?;
        let mut items = Vec::new();
        let mut next_cursor = None;
        for child in children {
            let Some(summary) = child.summary(caller)? else {
                continue;
            };
            if request.mode.is_some_and(|mode| summary.mode != mode)
                || request
                    .lifecycle
                    .is_some_and(|lifecycle| summary.lifecycle != lifecycle)
                || request
                    .cursor
                    .as_ref()
                    .is_some_and(|cursor| &summary.id <= cursor)
            {
                continue;
            }
            if request.limit == Some(items.len()) {
                next_cursor = items
                    .last()
                    .map(|item: &crate::SessionSummary| item.id.clone());
                break;
            }
            items.push(summary);
        }
        Ok(SessionListPage { items, next_cursor })
    }
}
