use crate::{
    ToolListRequest,
    deadline::Deadline,
    diagnostics::{Call, Correlation},
    operations::*,
    rpc::RpcClient,
};
use minicore_host_core::{
    AgentDetails, AgentListPage, AgentListRequest, AgentSummary, CallerContext, ConnectionHandle,
    EventSubscription, Host, HostConfig, HostConfigSnapshot, HostInfo, InteractionAnswerData,
    MessageSnapshot, ModelDescription, ModelTextSnapshot, ObservationNotification,
    SessionContextSnapshot, SessionHistorySnapshot, SessionInfo, SessionListPage,
    SessionListRequest, SessionSnapshot, SessionSummary, StateEvent, ToolCallSnapshot,
    ToolTextSnapshot, TurnDetailsSnapshot, TurnListSnapshot, TurnResultSnapshot, WorkspaceSnapshot,
};
use minicore_host_protocol::*;
use minicore_host_workspace::{
    ChangesListRequest, ChangesListResult, DiffRequest, DiffResult, FilesRequest, FilesResult,
    ReadRequest, ReadResult, SearchRequest, SearchResult, StatusResult,
};
use minicore_runtime::tools::ToolSpec;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::Instrument;

/// The existing Host business entrance. Its context comes from an actual
/// ConnectionHandle, never from serialized parameters or a claimed source.
#[derive(Clone)]
pub struct HostService {
    host: Arc<Host>,
}
impl HostService {
    pub fn new(host: Arc<Host>) -> Self {
        Self { host }
    }
}
impl MessageService<CallerContext> for HostService {
    async fn send(
        &self,
        caller: CallerContext,
        request: SendMessage,
    ) -> Result<MessageAccepted, Fault> {
        self.host.send(&caller, request)
    }
}

pub(crate) struct LocalConnection {
    host: Arc<Host>,
    handle: ConnectionHandle,
}
#[derive(Clone)]
enum Connection {
    Local(Arc<LocalConnection>),
    Rpc(Arc<RpcClient>),
}

/// Clones share one actual connection and its operation right. Creating a
/// SessionClient only bundles an address; it neither registers nor retains a
/// SessionController or Runner. Dropping the last client closes the connection.
#[derive(Clone)]
pub struct HostClient {
    connection: Connection,
}
impl HostClient {
    /// Trusted embedding entry. Business calls remain typed end to end.
    pub fn in_process(host: Arc<Host>) -> Result<Self, CallError> {
        let handle = host.connect()?;
        Ok(Self {
            connection: Connection::Local(Arc::new(LocalConnection { host, handle })),
        })
    }

    /// Initialize one trusted stdio channel. The caller supplies actual pipe or
    /// stream halves; this function does not spawn a process or authenticate a
    /// network peer. Initialization itself uses the same single wait budget.
    pub async fn stdio<R, W>(reader: R, writer: W, options: CallOptions) -> Result<Self, CallError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let deadline = Deadline::new(options)?;
        let connection = RpcClient::start(reader, writer);
        let mut diagnostic = Call::client(None, "initialize", Correlation::default());
        let span = diagnostic.span();
        let result = deadline
            .wait(connection.initialize(&mut diagnostic))
            .instrument(span)
            .await;
        diagnostic.client_finished(&result);
        let initial = result?;
        Ok(Self {
            connection: Connection::Rpc(Arc::new(connection.initialized(initial))),
        })
    }

    /// Establish TLS, WebSocket framing, and authenticated initialization using
    /// one monotonic wait budget. Certificate/name validation uses the supplied
    /// rustls configuration; credentials are sent only after TLS succeeds.
    /// No retry or reconnect is automatic, and plain `ws://` is rejected.
    pub async fn wss(
        url: &str,
        tls: Arc<crate::tls::ClientConfig>,
        access_key: &str,
        options: CallOptions,
    ) -> Result<Self, CallError> {
        let deadline = Deadline::new(options)?;
        let mut diagnostic = Call::client(None, "initialize", Correlation::default());
        let span = diagnostic.span();
        let result = deadline
            .wait(crate::wss::connect(url, tls, access_key, &mut diagnostic))
            .instrument(span)
            .await;
        diagnostic.client_finished(&result);
        let client = result?;
        Ok(Self {
            connection: Connection::Rpc(Arc::new(client)),
        })
    }

    /// Initialization facts, not a fresh Host state query or renewable identity.
    pub fn initial_info(&self) -> &ConnectionInfo {
        match &self.connection {
            Connection::Local(local) => local.handle.initial_info(),
            Connection::Rpc(rpc) => rpc.initial_info(),
        }
    }
    /// End this actual connection, including its clones and pending observations.
    /// Already accepted business work remains with the Host and direct owners.
    pub fn close(&self) {
        match &self.connection {
            Connection::Local(local) => local.handle.close(),
            Connection::Rpc(rpc) => rpc.close(),
        }
    }
    pub fn session(&self, target: SessionRef) -> SessionClient {
        SessionClient {
            client: self.clone(),
            target,
        }
    }
    async fn call<O: Operation>(
        &self,
        request: O,
        options: CallOptions,
        correlate: impl FnOnce(&O) -> Correlation,
    ) -> Result<O::Output, CallError> {
        let deadline = Deadline::new(options)?;
        let mut diagnostic =
            Call::client(Some(self.initial_info()), O::METHOD, correlate(&request));
        let span = diagnostic.span();
        let result = deadline
            .wait(async {
                match &self.connection {
                    Connection::Local(local) => request
                        .execute(Arc::clone(&local.host), local.handle.caller())
                        .await
                        .map_err(CallError::Business),
                    Connection::Rpc(rpc) => rpc.call(request, &mut diagnostic).await,
                }
            })
            .instrument(span)
            .await;
        // Every owner/admission scope ended before this forwarding summary.
        diagnostic.client_finished(&result);
        result
    }
    /// Only setup uses the call deadline. The established stream has no timer.
    pub async fn subscribe(
        &self,
        request: SubscribeRequest,
        options: CallOptions,
    ) -> Result<ClientSubscription, CallError> {
        let deadline = Deadline::new(options)?;
        let mut diagnostic = Call::client(
            Some(self.initial_info()),
            "events.subscribe",
            Correlation::subscription(&request.scope),
        );
        let span = diagnostic.span();
        let result = deadline
            .wait(async {
                match &self.connection {
                    Connection::Local(local) => {
                        let subscription = local
                            .host
                            .events_subscribe(&local.handle.caller(), request)?;
                        Ok(ClientSubscription {
                            inner: SubscriptionConnection::Local {
                                subscription: Box::new(subscription),
                                _connection: Arc::clone(local),
                            },
                        })
                    }
                    Connection::Rpc(rpc) => Ok(ClientSubscription {
                        inner: SubscriptionConnection::Rpc(
                            rpc.subscribe(request, &mut diagnostic).await?,
                        ),
                    }),
                }
            })
            .instrument(span)
            .await;
        diagnostic.client_finished(&result);
        result
    }
    pub async fn unsubscribe(
        &self,
        request: UnsubscribeRequest,
        options: CallOptions,
    ) -> Result<UnsubscribeResult, CallError> {
        let id = request.subscription_id.clone();
        let result = self
            .call(Unsubscribe(request), options, |_| Correlation::default())
            .await?;
        if let Connection::Rpc(rpc) = &self.connection {
            rpc.ended_subscription(&id);
        }
        Ok(result)
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

/// An address-only convenience context. Multiple Sessions can share a client
/// and issue concurrent calls without any server-side current-Session state.
#[derive(Clone)]
pub struct SessionClient {
    client: HostClient,
    target: SessionRef,
}
macro_rules! session_control {
    ($($name:ident, $operation:ident;)+) => {$(
        pub async fn $name(&self, request: SessionControlRequest, options: CallOptions) -> Result<SessionControlReceipt, CallError> {
            self.client.call($operation(SessionOperation { target: self.target.clone(), operation_id: request.operation_id }), options,
                |operation| Correlation::session(&operation.0.target).operation(&operation.0.operation_id)).await
        }
    )+};
}
impl SessionClient {
    pub fn target(&self) -> &SessionRef {
        &self.target
    }
    pub async fn send(
        &self,
        request: MessageInput,
        options: CallOptions,
    ) -> Result<MessageAccepted, CallError> {
        self.client
            .send(SendMessage::new(request, self.target.clone()), options)
            .await
    }
    pub async fn info(&self, options: CallOptions) -> Result<SessionInfo, CallError> {
        self.client
            .call(
                GetSession(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    /// Changes commit as one candidate. A running Turn keeps its old binding.
    pub async fn update(
        &self,
        request: SessionUpdateRequest,
        options: CallOptions,
    ) -> Result<SessionUpdateReceipt, CallError> {
        self.client
            .call(
                UpdateSession(UpdateSessionParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    expected_config_revision: request.expected_config_revision,
                    changes: request.changes,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    pub async fn inputs(&self, options: CallOptions) -> Result<InputList, CallError> {
        self.client
            .call(
                ListInputs(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn remove_input(
        &self,
        request: InputRemoveRequest,
        options: CallOptions,
    ) -> Result<InputRemoveReceipt, CallError> {
        self.client
            .call(
                RemoveInput(RemoveInputParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    message_id: request.message_id,
                    expected_controller_revision: request.expected_controller_revision,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.operation_id)
                        .message(&request.message_id, None)
                },
            )
            .await
    }
    /// Read only facts already held by the Session. Cold history stays unknown;
    /// this call does not load, prepare resources, or invoke a model.
    pub async fn context(&self, options: CallOptions) -> Result<SessionContextSnapshot, CallError> {
        self.client
            .call(
                GetSessionContext(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn state(&self, options: CallOptions) -> Result<SessionSnapshot, CallError> {
        self.client
            .call(
                SessionState(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    session_control! { pause, PauseSession; resume, ResumeSession; load, LoadSession; unload, UnloadSession; }
    /// Start manual compaction at the Session owner without creating a Turn or
    /// queuing a command. The receipt records acceptance, not completion. Read
    /// later progress from Session state; abandoning this wait does not cancel it.
    pub async fn compact(
        &self,
        request: CompactRequest,
        options: CallOptions,
    ) -> Result<CompactReceipt, CallError> {
        self.client
            .call(
                CompactSession(SessionOperation {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    /// Request cancellation of exactly the original manual operation. This
    /// does not clear either input queue, cancel a later operation, or promise
    /// rollback of an already committed history replacement.
    pub async fn cancel_compact(
        &self,
        request: CompactCancelRequest,
        options: CallOptions,
    ) -> Result<CompactCancelReceipt, CallError> {
        self.client
            .call(
                CancelCompact(CancelCompactParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    target_operation_id: request.target_operation_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.operation_id)
                        .target_operation(&request.target_operation_id)
                },
            )
            .await
    }
    pub async fn cancel(
        &self,
        request: SessionCancelRequest,
        options: CallOptions,
    ) -> Result<SessionCancelReceipt, CallError> {
        self.client
            .call(
                CancelSession(CancelSessionParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    keep_inbox: request.keep_inbox,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    pub async fn close(
        &self,
        request: SessionCloseRequest,
        options: CallOptions,
    ) -> Result<SessionCloseReceipt, CallError> {
        self.client
            .call(
                CloseSession(SessionOperation {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    /// Admission is separate from later close, worker settlement and file removal.
    pub async fn delete(
        &self,
        request: SessionDeleteRequest,
        options: CallOptions,
    ) -> Result<SessionDeleteReceipt, CallError> {
        self.client
            .call(
                DeleteSession(DeleteSessionParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    close: request.close,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    pub async fn message(
        &self,
        message_id: MessageId,
        options: CallOptions,
    ) -> Result<MessageSnapshot, CallError> {
        self.client
            .call(
                GetMessage(MessageTargetParams {
                    target: self.target.clone(),
                    message_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).message(&request.message_id, None)
                },
            )
            .await
    }
    pub async fn turns(&self, options: CallOptions) -> Result<TurnListSnapshot, CallError> {
        self.client
            .call(
                ListTurns(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn turn(
        &self,
        turn_id: TurnId,
        options: CallOptions,
    ) -> Result<TurnDetailsSnapshot, CallError> {
        self.client
            .call(
                GetTurn(TurnTarget {
                    target: self.target.clone(),
                    turn_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).turn(&request.turn_id)
                },
            )
            .await
    }
    pub async fn wait_turn(
        &self,
        turn_id: TurnId,
        options: CallOptions,
    ) -> Result<TurnResultSnapshot, CallError> {
        self.client
            .call(
                WaitTurn(TurnTarget {
                    target: self.target.clone(),
                    turn_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).turn(&request.turn_id)
                },
            )
            .await
    }
    pub async fn cancel_turn(
        &self,
        request: TurnCancelRequest,
        options: CallOptions,
    ) -> Result<TurnCancelReceipt, CallError> {
        self.client
            .call(
                CancelTurn(CancelTurnParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    turn_id: request.turn_id,
                    keep_inbox: request.keep_inbox,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.operation_id)
                        .turn(&request.turn_id)
                },
            )
            .await
    }
    pub async fn turn_result(
        &self,
        turn_id: TurnId,
        options: CallOptions,
    ) -> Result<TurnResultSnapshot, CallError> {
        self.client
            .call(
                GetTurnResult(TurnTarget {
                    target: self.target.clone(),
                    turn_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).turn(&request.turn_id)
                },
            )
            .await
    }
    pub async fn answer_interaction(
        &self,
        request: InteractionAnswerRequest<InteractionAnswerData>,
        options: CallOptions,
    ) -> Result<InteractionAnswerReceipt, CallError> {
        self.client
            .call(
                AnswerInteraction(AnswerParams {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.request.operation_id)
                        .turn(&request.request.turn_id)
                        .load(&request.request.load_id)
                },
            )
            .await
    }
    pub async fn turn_output(
        &self,
        turn_id: TurnId,
        response_index: u64,
        request: TextReadRequest,
        options: CallOptions,
    ) -> Result<ModelTextSnapshot, CallError> {
        self.client
            .call(
                ReadModelOutput(ModelReadParams {
                    target: self.target.clone(),
                    turn_id,
                    response_index,
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).turn(&request.turn_id)
                },
            )
            .await
    }
    pub async fn toolcall(
        &self,
        tool: ToolCallRef,
        options: CallOptions,
    ) -> Result<ToolCallSnapshot, CallError> {
        self.client
            .call(
                GetToolcall(ToolTargetParams {
                    target: self.target.clone(),
                    tool,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).tool(&request.tool)
                },
            )
            .await
    }
    pub async fn tool_output(
        &self,
        tool: ToolCallRef,
        request: TextReadRequest,
        options: CallOptions,
    ) -> Result<ToolTextSnapshot, CallError> {
        self.client
            .call(
                ReadToolOutput(ToolReadParams {
                    target: self.target.clone(),
                    tool,
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).tool(&request.tool)
                },
            )
            .await
    }
    /// Read only the explicitly addressed Session's committed workspace.
    pub async fn workspace_files(
        &self,
        request: FilesRequest,
        options: CallOptions,
    ) -> Result<WorkspaceSnapshot<FilesResult>, CallError> {
        self.client
            .call(
                WorkspaceFiles(FilesParams {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn workspace_read(
        &self,
        request: ReadRequest,
        options: CallOptions,
    ) -> Result<WorkspaceSnapshot<ReadResult>, CallError> {
        self.client
            .call(
                WorkspaceRead(WorkspaceReadParams {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn workspace_search(
        &self,
        request: SearchRequest,
        options: CallOptions,
    ) -> Result<WorkspaceSnapshot<SearchResult>, CallError> {
        self.client
            .call(
                WorkspaceSearch(SearchParams {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn changes_list(
        &self,
        request: ChangesListRequest,
        options: CallOptions,
    ) -> Result<WorkspaceSnapshot<ChangesListResult>, CallError> {
        self.client
            .call(
                ChangesList(ChangesListParams {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn changes_diff(
        &self,
        request: DiffRequest,
        options: CallOptions,
    ) -> Result<WorkspaceSnapshot<DiffResult>, CallError> {
        self.client
            .call(
                ChangesDiff(ChangesDiffParams {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
    pub async fn workspace_status(
        &self,
        options: CallOptions,
    ) -> Result<WorkspaceSnapshot<StatusResult>, CallError> {
        self.client
            .call(
                WorkspaceStatus(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }

    pub async fn history(
        &self,
        request: HistoryReadRequest,
        options: CallOptions,
    ) -> Result<SessionHistorySnapshot, CallError> {
        self.client
            .call(
                ReadHistory(HistoryParams {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                },
            )
            .await
    }
}

/// One observation on a shared connection. Dropping it ends only this stream,
/// never accepted work or another observer. It retains no Session or Runner.
pub struct ClientSubscription {
    inner: SubscriptionConnection,
}
enum SubscriptionConnection {
    Local {
        subscription: Box<EventSubscription>,
        _connection: Arc<LocalConnection>,
    },
    Rpc(crate::rpc::RpcSubscription),
}
impl ClientSubscription {
    pub fn result(&self) -> &SubscribeResult<StateEvent> {
        match &self.inner {
            SubscriptionConnection::Local { subscription, .. } => subscription.result(),
            SubscriptionConnection::Rpc(subscription) => subscription.result(),
        }
    }
    pub async fn recv(&mut self) -> Result<Option<ObservationNotification>, CallError> {
        match &mut self.inner {
            SubscriptionConnection::Local { subscription, .. } => {
                subscription.recv().await.map_err(CallError::Business)
            }
            SubscriptionConnection::Rpc(subscription) => subscription.recv().await,
        }
    }
}
