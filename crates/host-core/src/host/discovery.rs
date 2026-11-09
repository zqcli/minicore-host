use super::*;
use crate::{
    AgentListPage, AgentListRequest, AgentSummary, SessionListPage, SessionListRequest,
    SessionSummary,
};

impl Host {
    pub(crate) fn agent_summary(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<AgentSummary, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?.summary(caller)
    }

    pub fn agent_details(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<crate::AgentDetails, Fault> {
        self.check_host(&target.host_id)?;
        let agent = self.agent(caller, &target.agent_id)?;
        Ok(crate::AgentDetails {
            info: agent.info(caller)?,
            sessions: agent
                .session_page(caller, &SessionListRequest::default())?
                .items,
        })
    }

    pub(super) fn discovery_info(
        &self,
    ) -> (
        String,
        crate::HostCapabilities,
        crate::HostResourceSummary,
        crate::HostFacilities,
    ) {
        let storage_open = self
            .store
            .lock()
            .expect("Host store mutex poisoned")
            .is_some();
        let network_authentication = self.access.require_network_enabled().is_ok();
        let configuration = self
            .configuration
            .state
            .lock()
            .expect("Host configuration mutex poisoned");
        let current = &configuration.snapshot;
        let mut storage_modes = vec![StorageMode::Memory];
        if storage_open {
            storage_modes.push(StorageMode::Persistent);
        }
        (
            current.config.name.clone(),
            crate::HostCapabilities {
                storage_modes,
                configuration_reload: configuration.preparer.is_some(),
            },
            crate::HostResourceSummary {
                config_revision: current.revision,
                models: current.config.models.len(),
                tools: current.tools.len(),
            },
            crate::HostFacilities {
                storage_open,
                execution_resources: current.resources.is_some(),
                network_authentication,
            },
        )
    }

    /// Agent-ID lexical order; the cursor is exclusive even if its object was
    /// subsequently deleted. A missing limit returns every remaining Agent.
    pub fn list_agents(
        &self,
        caller: &CallerContext,
        request: &AgentListRequest,
    ) -> Result<AgentListPage, Fault> {
        crate::discovery::validate_list_limit(request.limit)?;
        let agents = self.access.read(caller, || {
            let state = self.state.lock().expect("Host mutex poisoned");
            Ok(state.agents.values().cloned().collect::<Vec<_>>())
        })?;
        let mut items = Vec::new();
        let mut next_cursor = None;
        for agent in agents {
            if request
                .cursor
                .as_ref()
                .is_some_and(|cursor| &agent.id <= cursor)
            {
                continue;
            }
            if request.limit == Some(items.len()) {
                next_cursor = items.last().map(|item: &AgentSummary| item.id.clone());
                break;
            }
            items.push(agent.summary(caller)?);
        }
        Ok(AgentListPage { items, next_cursor })
    }

    /// All current lightweight Agent summaries, through the same list path.
    pub fn agents(&self, caller: &CallerContext) -> Result<Vec<AgentSummary>, Fault> {
        Ok(self
            .list_agents(caller, &AgentListRequest::default())?
            .items)
    }

    pub fn list_sessions(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        request: &SessionListRequest,
    ) -> Result<SessionListPage, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .session_page(caller, request)
    }

    /// All current lightweight Session summaries, through the same list path.
    pub fn sessions(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<Vec<SessionSummary>, Fault> {
        Ok(self
            .list_sessions(caller, target, &SessionListRequest::default())?
            .items)
    }
}
