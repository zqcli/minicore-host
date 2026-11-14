//! Host and Agent calls over the client's existing connection.
use super::{SessionClient, connection::Connection};
use crate::{diagnostics::Correlation, operations::*};
use minicore_host_core::{
    AgentDetails, AgentListPage, AgentListRequest, AgentSummary, HostConfig, HostConfigSnapshot,
    HostInfo, ModelDescription, SessionListPage, SessionListRequest, SessionSummary,
};
use minicore_host_protocol::*;
use minicore_host_runtime::tools::ToolSpec;

/// Clones share one actual connection and its operation right. Creating a
/// SessionClient only bundles an address; it neither registers nor retains a
/// SessionController or Runner. Dropping the last client closes the connection.
#[derive(Clone)]
pub struct HostClient {
    pub(super) connection: Connection,
}
impl HostClient {
    pub fn session(&self, target: SessionRef) -> SessionClient {
        SessionClient {
            client: self.clone(),
            target,
        }
    }
    pub async fn info(&self, options: CallOptions) -> Result<HostInfo, CallError> {
        self.call(Info(Empty {}), options, |_| Correlation::default())
            .await
    }
    /// Read committed values and credential source names, never private keys
    /// or executable resource handles. Available to read-only connections.
    pub async fn config(&self, options: CallOptions) -> Result<HostConfigSnapshot, CallError> {
        self.call(GetConfig(Empty {}), options, |_| Correlation::default())
            .await
    }
    /// Accept a full candidate at the Host configuration owner. The returned
    /// receipt is a snapshot, not a wait for publication. Read later facts with
    /// `receipt`; timeout or connection loss does not cancel an accepted reload.
    pub async fn reload(
        &self,
        request: HostReloadRequest<HostConfig>,
        options: CallOptions,
    ) -> Result<HostReloadReceipt, CallError> {
        self.call(Reload(request), options, |operation| {
            let request = &operation.0;
            Correlation::default().operation(&request.operation_id)
        })
        .await
    }
    /// List configured models by their full reference in lexical order.
    pub async fn models(&self, options: CallOptions) -> Result<Vec<ModelDescription>, CallError> {
        self.call(ListModels(Empty {}), options, |_| Correlation::default())
            .await
    }
    pub async fn workspace_roots(&self, options: CallOptions) -> Result<DirectoryRoots, CallError> {
        self.call(WorkspaceRoots(Empty {}), options, |_| {
            Correlation::default()
        })
        .await
    }
    pub async fn workspace_list(
        &self,
        request: DirectoryListRequest,
        options: CallOptions,
    ) -> Result<DirectoryListResult, CallError> {
        self.call(WorkspaceList(request), options, |_| Correlation::default())
            .await
    }
    pub async fn workspace_mkdir(
        &self,
        request: DirectoryMkdirRequest,
        options: CallOptions,
    ) -> Result<DirectoryMkdirReceipt, CallError> {
        self.call(WorkspaceMkdir(request), options, |operation| {
            Correlation::default().operation(&operation.0.operation_id)
        })
        .await
    }
    /// List the Host's available tools or an explicit owner's enabled set.
    /// Missing targets or selected resources preserve the owner's Fault.
    pub async fn tools(
        &self,
        request: ToolListRequest,
        options: CallOptions,
    ) -> Result<Vec<ToolSpec>, CallError> {
        self.call(ListTools(request), options, |operation| {
            let request = &operation.0;
            match request.target.as_ref() {
                Some(crate::ToolListTarget::Agent(target)) => Correlation::agent(target),
                Some(crate::ToolListTarget::Session(target)) => Correlation::session(target),
                None => Correlation::default(),
            }
        })
        .await
    }
    pub async fn receipt(
        &self,
        operation_id: OperationId,
        options: CallOptions,
    ) -> Result<Receipt, CallError> {
        self.call(
            ReceiptGet(OperationTarget { operation_id }),
            options,
            |operation| {
                let request = &operation.0;
                Correlation::default().operation(&request.operation_id)
            },
        )
        .await
    }
    pub async fn shutdown(
        &self,
        request: HostShutdownRequest,
        options: CallOptions,
    ) -> Result<HostShutdownReceipt, CallError> {
        self.call(Shutdown(request), options, |operation| {
            let request = &operation.0;
            Correlation::default().operation(&request.operation_id)
        })
        .await
    }
    pub async fn create_agent(
        &self,
        request: AgentCreate,
        options: CallOptions,
    ) -> Result<AgentCreateReceipt, CallError> {
        self.call(CreateAgent(request), options, |operation| {
            let request = &operation.0;
            Correlation::default()
                .operation(&request.operation_id)
                .initial_message(request.initial_message.as_ref())
        })
        .await
    }
    pub async fn agent(
        &self,
        target: AgentRef,
        options: CallOptions,
    ) -> Result<AgentDetails, CallError> {
        self.call(GetAgent(AgentTarget { target }), options, |operation| {
            let request = &operation.0;
            Correlation::agent(&request.target)
        })
        .await
    }
    pub async fn agents(&self, options: CallOptions) -> Result<Vec<AgentSummary>, CallError> {
        Ok(self
            .list_agents(AgentListRequest::default(), options)
            .await?
            .items)
    }
    pub async fn list_agents(
        &self,
        request: AgentListRequest,
        options: CallOptions,
    ) -> Result<AgentListPage, CallError> {
        self.call(ListAgents(request), options, |_| Correlation::default())
            .await
    }
    pub async fn stop_agent(
        &self,
        target: AgentRef,
        request: AgentStopRequest,
        options: CallOptions,
    ) -> Result<AgentStopReceipt, CallError> {
        self.call(
            StopAgent(AgentOperation {
                target,
                operation_id: request.operation_id,
            }),
            options,
            |operation| {
                let request = &operation.0;
                Correlation::agent(&request.target).operation(&request.operation_id)
            },
        )
        .await
    }
    /// Enable only this Agent's scheduling. Explicit child pauses remain intact.
    pub async fn start_agent(
        &self,
        target: AgentRef,
        request: AgentStartRequest,
        options: CallOptions,
    ) -> Result<AgentStartReceipt, CallError> {
        self.call(
            StartAgent(AgentOperation {
                target,
                operation_id: request.operation_id,
            }),
            options,
            |operation| {
                let request = &operation.0;
                Correlation::agent(&request.target).operation(&request.operation_id)
            },
        )
        .await
    }
    /// Persistent metadata is accepted as saving, not yet committed. Inspect
    /// agent.get for completion; the original receipt stays unchanged.
    pub async fn update_agent(
        &self,
        target: AgentRef,
        request: AgentUpdateRequest,
        options: CallOptions,
    ) -> Result<AgentUpdateReceipt, CallError> {
        self.call(
            UpdateAgent(UpdateAgentParams {
                target,
                operation_id: request.operation_id,
                expected_config_revision: request.expected_config_revision,
                changes: request.changes,
            }),
            options,
            |operation| {
                let request = &operation.0;
                Correlation::agent(&request.target).operation(&request.operation_id)
            },
        )
        .await
    }
    pub async fn set_default_session(
        &self,
        target: AgentRef,
        request: AgentSetDefaultSessionRequest,
        options: CallOptions,
    ) -> Result<AgentSetDefaultSessionReceipt, CallError> {
        self.call(
            SetDefaultSession(SetDefaultSessionParams {
                target,
                operation_id: request.operation_id,
                session_id: request.session_id,
            }),
            options,
            |operation| {
                let request = &operation.0;
                Correlation::agent(&request.target)
                    .operation(&request.operation_id)
                    .session_id(request.session_id.as_ref())
            },
        )
        .await
    }
    /// Delete only explicitly included owner data, after its lifecycle boundary.
    pub async fn delete_agent(
        &self,
        target: AgentRef,
        request: AgentDeleteRequest,
        options: CallOptions,
    ) -> Result<AgentDeleteReceipt, CallError> {
        self.call(
            DeleteAgent(DeleteAgentParams {
                target,
                operation_id: request.operation_id,
                scope: request.scope,
            }),
            options,
            |operation| {
                let request = &operation.0;
                Correlation::agent(&request.target).operation(&request.operation_id)
            },
        )
        .await
    }
    pub async fn create_session(
        &self,
        request: SessionCreate,
        options: CallOptions,
    ) -> Result<SessionCreateReceipt, CallError> {
        self.call(CreateSession(request), options, |operation| {
            let request = &operation.0;
            Correlation::agent(&request.target)
                .operation(&request.operation_id)
                .initial_message(request.initial_message.as_ref())
        })
        .await
    }
    pub async fn sessions(
        &self,
        target: AgentRef,
        options: CallOptions,
    ) -> Result<Vec<SessionSummary>, CallError> {
        Ok(self
            .list_sessions(target, SessionListRequest::default(), options)
            .await?
            .items)
    }
    pub async fn list_sessions(
        &self,
        target: AgentRef,
        query: SessionListRequest,
        options: CallOptions,
    ) -> Result<SessionListPage, CallError> {
        self.call(
            ListSessions(ListSessionsParams { target, query }),
            options,
            |operation| {
                let request = &operation.0;
                Correlation::agent(&request.target)
            },
        )
        .await
    }
    pub async fn send(
        &self,
        request: SendMessage,
        options: CallOptions,
    ) -> Result<MessageAccepted, CallError> {
        self.call(SendMessageOp(request), options, |operation| {
            let request = &operation.0;
            Correlation::send(request)
        })
        .await
    }
}
impl MessageClient for HostClient {
    async fn send(
        &self,
        request: SendMessage,
        options: CallOptions,
    ) -> Result<MessageAccepted, CallError> {
        HostClient::send(self, request, options).await
    }
}
