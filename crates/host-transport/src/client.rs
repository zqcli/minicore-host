use crate::{deadline::Deadline, operations::*, rpc::RpcClient};
use minicore_host_core::{
    AgentInfo, CallerContext, ConnectionHandle, Host, HostInfo, MessageRecord, SessionInfo,
    SessionSnapshot, TurnInfo, TurnResult,
};
use minicore_host_protocol::*;
use minicore_runtime::HistoryItem;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

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

struct LocalConnection {
    host: Arc<Host>,
    handle: ConnectionHandle,
}
#[derive(Clone)]
enum Connection {
    Local(Arc<LocalConnection>),
    Stdio(Arc<RpcClient>),
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
        let initial = deadline.wait(connection.initialize()).await?;
        Ok(Self {
            connection: Connection::Stdio(Arc::new(connection.initialized(initial))),
        })
    }

    /// Initialization facts, not a fresh Host state query or renewable identity.
    pub fn initial_info(&self) -> &ConnectionInfo {
        match &self.connection {
            Connection::Local(local) => local.handle.initial_info(),
            Connection::Stdio(rpc) => rpc.initial_info(),
        }
    }
    /// End this actual connection, including its clones and pending observations.
    /// Already accepted business work remains with the Host and direct owners.
    pub fn close(&self) {
        match &self.connection {
            Connection::Local(local) => local.handle.close(),
            Connection::Stdio(rpc) => rpc.close(),
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
    ) -> Result<O::Output, CallError> {
        let deadline = Deadline::new(options)?;
        deadline
            .wait(async {
                match &self.connection {
                    Connection::Local(local) => request
                        .execute(Arc::clone(&local.host), local.handle.caller())
                        .await
                        .map_err(CallError::Business),
                    Connection::Stdio(rpc) => rpc.call(request).await,
                }
            })
            .await
    }
    pub async fn info(&self, options: CallOptions) -> Result<HostInfo, CallError> {
        self.call(Info(Empty {}), options).await
    }
    pub async fn receipt(
        &self,
        operation_id: OperationId,
        options: CallOptions,
    ) -> Result<Receipt, CallError> {
        self.call(ReceiptGet(OperationTarget { operation_id }), options)
            .await
    }
    pub async fn shutdown(
        &self,
        request: HostShutdownRequest,
        options: CallOptions,
    ) -> Result<HostShutdownReceipt, CallError> {
        self.call(Shutdown(request), options).await
    }
    pub async fn create_agent(
        &self,
        request: AgentCreate,
        options: CallOptions,
    ) -> Result<AgentCreateReceipt, CallError> {
        self.call(CreateAgent(request), options).await
    }
    pub async fn agent(
        &self,
        target: AgentRef,
        options: CallOptions,
    ) -> Result<AgentInfo, CallError> {
        self.call(GetAgent(AgentTarget { target }), options).await
    }
    pub async fn agents(&self, options: CallOptions) -> Result<Vec<AgentInfo>, CallError> {
        self.call(ListAgents(Empty {}), options).await
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
        )
        .await
    }
    pub async fn create_session(
        &self,
        request: SessionCreate,
        options: CallOptions,
    ) -> Result<SessionCreateReceipt, CallError> {
        self.call(CreateSession(request), options).await
    }
    pub async fn sessions(
        &self,
        target: AgentRef,
        options: CallOptions,
    ) -> Result<Vec<SessionInfo>, CallError> {
        self.call(ListSessions(AgentTarget { target }), options)
            .await
    }
    pub async fn send(
        &self,
        request: SendMessage,
        options: CallOptions,
    ) -> Result<MessageAccepted, CallError> {
        self.call(SendMessageOp(request), options).await
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
            self.client.call($operation(SessionOperation { target: self.target.clone(), operation_id: request.operation_id }), options).await
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
            )
            .await
    }
    session_control! { pause, PauseSession; resume, ResumeSession; load, LoadSession; unload, UnloadSession; }
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
            )
            .await
    }
    pub async fn message(
        &self,
        message_id: MessageId,
        options: CallOptions,
    ) -> Result<MessageRecord, CallError> {
        self.client
            .call(
                GetMessage(MessageTargetParams {
                    target: self.target.clone(),
                    message_id,
                }),
                options,
            )
            .await
    }
    pub async fn turns(&self, options: CallOptions) -> Result<Vec<TurnInfo>, CallError> {
        self.client
            .call(
                ListTurns(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
            )
            .await
    }
    pub async fn turn(&self, turn_id: TurnId, options: CallOptions) -> Result<TurnInfo, CallError> {
        self.client
            .call(
                GetTurn(TurnTarget {
                    target: self.target.clone(),
                    turn_id,
                }),
                options,
            )
            .await
    }
    pub async fn wait_turn(
        &self,
        turn_id: TurnId,
        options: CallOptions,
    ) -> Result<TurnResult, CallError> {
        self.client
            .call(
                WaitTurn(TurnTarget {
                    target: self.target.clone(),
                    turn_id,
                }),
                options,
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
            )
            .await
    }
    pub async fn turn_result(
        &self,
        turn_id: TurnId,
        options: CallOptions,
    ) -> Result<TurnResult, CallError> {
        self.client
            .call(
                GetTurnResult(TurnTarget {
                    target: self.target.clone(),
                    turn_id,
                }),
                options,
            )
            .await
    }
    pub async fn history(&self, options: CallOptions) -> Result<Vec<HistoryItem>, CallError> {
        self.client
            .call(
                ReadHistory(SessionTarget {
                    target: self.target.clone(),
                }),
                options,
            )
            .await
    }
}
