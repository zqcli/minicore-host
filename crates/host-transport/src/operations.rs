//! One typed operation table drives direct calls and RPC dispatch.
use minicore_host_core::{
    AgentDetails, AgentListPage, AgentListRequest, CallerContext, Host, HostConfigSnapshot,
    HostInfo, InteractionAnswerData, MessageSnapshot, ModelDescription, ModelTextSnapshot,
    SessionContextSnapshot, SessionHistorySnapshot, SessionInfo, SessionListPage,
    SessionListRequest, SessionSnapshot, ToolCallSnapshot, ToolTextSnapshot, TurnDetailsSnapshot,
    TurnListSnapshot, TurnResultSnapshot, WorkspaceSnapshot,
};
use minicore_host_protocol::*;
use minicore_host_runtime::tools::ToolSpec;
use minicore_host_workspace::{
    ChangesListRequest, ChangesListResult, DiffRequest, DiffResult, FilesRequest, FilesResult,
    QueryContext, ReadRequest, ReadResult, SearchRequest, SearchResult, StatusResult,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{future::Future, sync::Arc};

pub(crate) trait Operation: Serialize + DeserializeOwned + Send + 'static {
    const METHOD: Method;
    type Output: Serialize + DeserializeOwned + Send + 'static;
    fn execute(
        self,
        host: Arc<Host>,
        caller: CallerContext,
    ) -> impl Future<Output = Result<Self::Output, Fault>> + Send;
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Empty {}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentTarget {
    pub target: AgentRef,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListSessionsParams {
    pub target: AgentRef,
    #[serde(flatten)]
    pub query: SessionListRequest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionTarget {
    pub target: SessionRef,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationTarget {
    pub operation_id: OperationId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentOperation {
    pub target: AgentRef,
    pub operation_id: OperationId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionOperation {
    pub target: SessionRef,
    pub operation_id: OperationId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageTargetParams {
    pub target: SessionRef,
    pub message_id: MessageId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TurnTarget {
    pub target: SessionRef,
    pub turn_id: TurnId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelTurnParams {
    pub target: SessionRef,
    pub operation_id: OperationId,
    pub turn_id: TurnId,
    #[serde(default)]
    pub keep_inbox: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelSessionParams {
    pub target: SessionRef,
    pub operation_id: OperationId,
    #[serde(default)]
    pub keep_inbox: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelCompactParams {
    pub target: SessionRef,
    pub operation_id: OperationId,
    pub target_operation_id: OperationId,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRequest<T> {
    pub target: SessionRef,
    pub request: T,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelReadParams {
    pub target: SessionRef,
    pub turn_id: TurnId,
    pub response_index: u64,
    pub request: TextReadRequest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolTargetParams {
    pub target: SessionRef,
    pub tool: ToolCallRef,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolReadParams {
    pub target: SessionRef,
    pub tool: ToolCallRef,
    pub request: TextReadRequest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteAgentParams {
    pub target: AgentRef,
    pub operation_id: OperationId,
    #[serde(default)]
    pub scope: AgentDeleteScope,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteSessionParams {
    pub target: SessionRef,
    pub operation_id: OperationId,
    #[serde(default)]
    pub close: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateAgentParams {
    pub target: AgentRef,
    pub operation_id: OperationId,
    pub expected_config_revision: ConfigRevision,
    pub changes: AgentUpdateChanges,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateSessionParams {
    pub target: SessionRef,
    pub operation_id: OperationId,
    pub expected_config_revision: ConfigRevision,
    pub changes: SessionUpdateChanges,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetDefaultSessionParams {
    pub target: AgentRef,
    pub operation_id: OperationId,
    #[serde(deserialize_with = "required_nullable")]
    pub session_id: Option<SessionId>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoveInputParams {
    pub target: SessionRef,
    pub operation_id: OperationId,
    pub message_id: MessageId,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub expected_controller_revision: Option<ControllerRevision>,
}
fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

macro_rules! operations {
    ($($name:ident, $method:ident, $params:ty => $output:ty, |$host:ident, $caller:ident, $p:ident| $body:block;)+) => {
        $(#[derive(Serialize, Deserialize)]
        #[serde(transparent)]
        pub(crate) struct $name(pub $params);
        impl Operation for $name {
            const METHOD: Method = Method::$method;
            type Output = $output;
            async fn execute(self, $host: Arc<Host>, $caller: CallerContext) -> Result<$output, Fault> {
                let $p = self.0;
                $body
            }
        })+

        /// Canonical resource methods backed by this adapter's real handlers.
        /// This route list is not a runtime capability negotiation.
        pub const SUPPORTED_METHODS: &[&str] = &[Method::EventsSubscribe.as_str(), $(Method::$method.as_str(),)+];

        pub(crate) async fn dispatch(host: Arc<Host>, caller: CallerContext, method: &str, params: Value, id: &RpcId) -> Result<Vec<u8>, RpcError> {
            match method.parse::<Method>() {
                $(Ok(Method::$method) => run::<$name>(host, caller, params, id).await,)+
                Ok(_) => Err(RpcError::business(Fault::new(
                    FaultKind::HostMethodUnavailable, "this method is not implemented by the current adapter",
                ))),
                Err(_) => Err(protocol_error(-32601, "Method not found")),
            }
        }
    };
}

operations! {
    Info, HostInfo, Empty => HostInfo, |h, c, _p| { h.info(&c) };
    GetConfig, HostConfigGet, Empty => HostConfigSnapshot, |h, c, _p| { h.config_get(&c) };
    Reload, HostReload, HostReloadRequest => HostReloadReceipt, |h, c, p| { h.reload(&c, p) };
    ListModels, ModelList, Empty => Vec<ModelDescription>, |h, c, _p| { h.model_list(&c) };
    WorkspaceRoots, WorkspaceRoots, Empty => DirectoryRoots, |h, c, _p| { h.workspace_roots(&c) };
    WorkspaceList, WorkspaceList, DirectoryListRequest => DirectoryListResult, |h, c, p| { h.workspace_list(&c, p).await };
    WorkspaceMkdir, WorkspaceMkdir, DirectoryMkdirRequest => DirectoryMkdirReceipt, |h, c, p| { h.workspace_mkdir(&c, p).await };
    ListTools, ToolList, ToolListRequest => Vec<ToolSpec>, |h, c, p| {
        match p.target {
            None => h.tool_list(&c),
            Some(ToolListTarget::Agent(target)) => h.tool_list_for_agent(&c, &target),
            Some(ToolListTarget::Session(target)) => h.tool_list_for_session(&c, &target),
        }
    };
    ReceiptGet, HostReceiptGet, OperationTarget => Receipt, |h, c, p| { h.receipt_get(&c, &p.operation_id) };
    Shutdown, HostShutdown, HostShutdownRequest => HostShutdownReceipt, |h, c, p| { h.shutdown(&c, p) };
    CreateAgent, AgentCreate, AgentCreate => AgentCreateReceipt, |h, c, p| { h.create_agent_async(&c, p).await };
    GetAgent, AgentGet, AgentTarget => AgentDetails, |h, c, p| { h.agent_details(&c, &p.target) };
    ListAgents, AgentList, AgentListRequest => AgentListPage, |h, c, p| { h.list_agents(&c, &p) };
    StartAgent, AgentStart, AgentOperation => AgentStartReceipt, |h, c, p| { h.agent_start(&c, &p.target, AgentStartRequest { operation_id: p.operation_id }) };
    UpdateAgent, AgentUpdate, UpdateAgentParams => AgentUpdateReceipt, |h, c, p| { h.agent_update(&c, &p.target, AgentUpdateRequest { operation_id: p.operation_id, expected_config_revision: p.expected_config_revision, changes: p.changes }).await };
    SetDefaultSession, AgentSetDefaultSession, SetDefaultSessionParams => AgentSetDefaultSessionReceipt, |h, c, p| { h.agent_set_default_session(&c, &p.target, AgentSetDefaultSessionRequest { operation_id: p.operation_id, session_id: p.session_id }) };
    StopAgent, AgentStop, AgentOperation => AgentStopReceipt, |h, c, p| { h.agent_stop(&c, &p.target, AgentStopRequest { operation_id: p.operation_id }) };
    DeleteAgent, AgentDelete, DeleteAgentParams => AgentDeleteReceipt, |h, c, p| { h.agent_delete(&c, &p.target, AgentDeleteRequest { operation_id: p.operation_id, scope: p.scope }) };
    CreateSession, SessionCreate, SessionCreate => SessionCreateReceipt, |h, c, p| { h.create_session_async(&c, p).await };
    GetSession, SessionGet, SessionTarget => SessionInfo, |h, c, p| { h.session_info(&c, &p.target) };
    GetSessionContext, SessionContext, SessionTarget => SessionContextSnapshot, |h, c, p| { h.session_context(&c, &p.target).await };
    ListSessions, SessionList, ListSessionsParams => SessionListPage, |h, c, p| { h.list_sessions(&c, &p.target, &p.query) };
    UpdateSession, SessionUpdate, UpdateSessionParams => SessionUpdateReceipt, |h, c, p| { h.session_update(&c, &p.target, SessionUpdateRequest { operation_id: p.operation_id, expected_config_revision: p.expected_config_revision, changes: p.changes }).await };
    ListInputs, InputList, SessionTarget => InputList, |h, c, p| { h.input_list(&c, &p.target) };
    RemoveInput, InputRemove, RemoveInputParams => InputRemoveReceipt, |h, c, p| { h.input_remove(&c, &p.target, InputRemoveRequest { operation_id: p.operation_id, message_id: p.message_id, expected_controller_revision: p.expected_controller_revision }) };
    SessionState, SessionState, SessionTarget => SessionSnapshot, |h, c, p| { h.session_state(&c, &p.target) };
    LoadSession, SessionLoad, SessionOperation => SessionControlReceipt, |h, c, p| { h.session_load(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    UnloadSession, SessionUnload, SessionOperation => SessionControlReceipt, |h, c, p| { h.session_unload(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    PauseSession, SessionPause, SessionOperation => SessionControlReceipt, |h, c, p| { h.session_pause(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    ResumeSession, SessionResume, SessionOperation => SessionControlReceipt, |h, c, p| { h.session_resume(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    CancelSession, SessionCancel, CancelSessionParams => SessionCancelReceipt, |h, c, p| { h.session_cancel(&c, &p.target, SessionCancelRequest { operation_id: p.operation_id, keep_inbox: p.keep_inbox }) };
    CloseSession, SessionClose, SessionOperation => SessionCloseReceipt, |h, c, p| { h.session_close(&c, &p.target, SessionCloseRequest { operation_id: p.operation_id }) };
    DeleteSession, SessionDelete, DeleteSessionParams => SessionDeleteReceipt, |h, c, p| { h.session_delete(&c, &p.target, SessionDeleteRequest { operation_id: p.operation_id, close: p.close }) };
    CompactSession, SessionCompact, SessionOperation => CompactReceipt, |h, c, p| { h.session_compact(&c, &p.target, CompactRequest { operation_id: p.operation_id }) };
    CancelCompact, SessionCompactCancel, CancelCompactParams => CompactCancelReceipt, |h, c, p| { h.session_compact_cancel(&c, &p.target, CompactCancelRequest { operation_id: p.operation_id, target_operation_id: p.target_operation_id }) };
    SendMessageOp, MessageSend, SendMessage => MessageAccepted, |h, c, p| { h.send(&c, p) };
    GetMessage, MessageGet, MessageTargetParams => MessageSnapshot, |h, c, p| { h.message_snapshot(&c, &p.target, &p.message_id) };
    GetTurn, TurnGet, TurnTarget => TurnDetailsSnapshot, |h, c, p| { h.turn_snapshot(&c, &p.target, &p.turn_id) };
    ListTurns, TurnList, SessionTarget => TurnListSnapshot, |h, c, p| { h.turn_list(&c, &p.target) };
    WaitTurn, TurnWait, TurnTarget => TurnResultSnapshot, |h, c, p| { h.wait_turn_snapshot(&c, &p.target, &p.turn_id).await };
    CancelTurn, TurnCancel, CancelTurnParams => TurnCancelReceipt, |h, c, p| { h.turn_cancel(&c, &p.target, TurnCancelRequest { operation_id: p.operation_id, turn_id: p.turn_id, keep_inbox: p.keep_inbox }) };
    GetTurnResult, TurnResult, TurnTarget => TurnResultSnapshot, |h, c, p| { h.turn_result_snapshot(&c, &p.target, &p.turn_id).await };
    ReadHistory, SessionHistoryRead, SessionRequest<HistoryReadRequest> => SessionHistorySnapshot, |h, c, p| { h.history_read(&c, &p.target, &p.request).await };
    ReadModelOutput, TurnOutputRead, ModelReadParams => ModelTextSnapshot, |h, c, p| { h.turn_output_read(&c, &p.target, &p.turn_id, p.response_index, &p.request) };
    GetToolcall, ToolcallGet, ToolTargetParams => ToolCallSnapshot, |h, c, p| { h.toolcall_get(&c, &p.target, &p.tool) };
    ReadToolOutput, ToolcallOutputRead, ToolReadParams => ToolTextSnapshot, |h, c, p| { h.toolcall_output_read(&c, &p.target, &p.tool, &p.request) };
    WorkspaceFiles, WorkspaceFiles, SessionRequest<FilesRequest> => WorkspaceSnapshot<FilesResult>, |h, c, p| { h.workspace_files(&c, &p.target, p.request, QueryContext::default()).await };
    WorkspaceRead, WorkspaceRead, SessionRequest<ReadRequest> => WorkspaceSnapshot<ReadResult>, |h, c, p| { h.workspace_read(&c, &p.target, p.request, QueryContext::default()).await };
    WorkspaceSearch, WorkspaceSearch, SessionRequest<SearchRequest> => WorkspaceSnapshot<SearchResult>, |h, c, p| { h.workspace_search(&c, &p.target, p.request, QueryContext::default()).await };
    ChangesList, ChangesList, SessionRequest<ChangesListRequest> => WorkspaceSnapshot<ChangesListResult>, |h, c, p| { h.changes_list(&c, &p.target, p.request, QueryContext::default()).await };
    ChangesDiff, ChangesDiff, SessionRequest<DiffRequest> => WorkspaceSnapshot<DiffResult>, |h, c, p| { h.changes_diff(&c, &p.target, p.request, QueryContext::default()).await };
    WorkspaceStatus, WorkspaceStatus, SessionTarget => WorkspaceSnapshot<StatusResult>, |h, c, p| { h.workspace_status(&c, &p.target, QueryContext::default()).await };
    AnswerInteraction, InteractionAnswer, SessionRequest<InteractionAnswerRequest<InteractionAnswerData>> => InteractionAnswerReceipt, |h, c, p| { h.answer_interaction(&c, &p.target, p.request) };
    Unsubscribe, EventsUnsubscribe, UnsubscribeRequest => UnsubscribeResult, |h, c, p| { h.events_unsubscribe(&c, &p) };
}

async fn run<O: Operation>(
    host: Arc<Host>,
    caller: CallerContext,
    params: Value,
    id: &RpcId,
) -> Result<Vec<u8>, RpcError> {
    let request: O = serde_json::from_value(params)
        .map_err(|_| protocol_error(-32602, "Invalid method parameters"))?;
    let result = request
        .execute(host, caller)
        .await
        .map_err(RpcError::business)?;
    encode_success(id, result)
}

pub(crate) fn encode_success(id: &RpcId, result: impl Serialize) -> Result<Vec<u8>, RpcError> {
    // Finish encoding before reporting success or queuing a frame. A failed
    // result drops its partial bytes and keeps the original protocol error.
    serde_json::to_vec(&RpcResponse::success(id.clone(), result))
        .map_err(|_| protocol_error(-32603, "Unable to encode the business result"))
}

pub(crate) fn protocol_error(code: i64, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}
