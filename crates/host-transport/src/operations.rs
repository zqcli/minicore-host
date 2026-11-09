//! One typed operation table drives direct calls and RPC dispatch.
use crate::resources::{ToolListRequest, ToolListTarget};
use minicore_host_core::{
    AgentInfo, CallerContext, Host, HostConfig, HostConfigSnapshot, HostInfo,
    InteractionAnswerData, MessageRecord, ModelDescription, ModelTextSnapshot,
    SessionHistorySnapshot, SessionInfo, SessionSnapshot, ToolCallSnapshot, ToolTextSnapshot,
    TurnInfo, TurnResult, TurnSnapshot,
};
use minicore_host_protocol::*;
use minicore_runtime::tools::ToolSpec;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{future::Future, sync::Arc};

pub(crate) trait Operation: Serialize + DeserializeOwned + Send + 'static {
    const METHOD: &'static str;
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
pub(crate) struct HistoryParams {
    pub target: SessionRef,
    pub request: HistoryReadRequest,
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
pub(crate) struct AnswerParams {
    pub target: SessionRef,
    pub request: InteractionAnswerRequest<InteractionAnswerData>,
}

macro_rules! operations {
    ($($name:ident, $method:literal, $params:ty => $output:ty, |$host:ident, $caller:ident, $p:ident| $body:block;)+) => {
        $(#[derive(Serialize, Deserialize)]
        #[serde(transparent)]
        pub(crate) struct $name(pub $params);
        impl Operation for $name {
            const METHOD: &'static str = $method;
            type Output = $output;
            async fn execute(self, $host: Arc<Host>, $caller: CallerContext) -> Result<$output, Fault> {
                let $p = self.0;
                $body
            }
        })+

        /// Resource methods with real handlers in this adapter increment.
        /// This is not the full SPEC §8 implementation or a runtime negotiation.
        pub const SUPPORTED_METHODS: &[&str] = &["events.subscribe", $($method,)+];

        pub(crate) async fn dispatch(host: Arc<Host>, caller: CallerContext, method: &str, params: Value) -> Result<Value, RpcError> {
            match method {
                $($method => run::<$name>(host, caller, params).await,)+
                _ if method.parse::<Method>().is_ok() => Err(RpcError::business(Fault::new(
                    FaultKind::HostMethodUnavailable, "this method is not implemented by the current adapter",
                ))),
                _ => Err(protocol_error(-32601, "Method not found")),
            }
        }
    };
}

operations! {
    Info, "host.info", Empty => HostInfo, |h, c, _p| { h.info(&c) };
    GetConfig, "host.config.get", Empty => HostConfigSnapshot, |h, c, _p| { h.config_get(&c) };
    Reload, "host.reload", HostReloadRequest<HostConfig> => HostReloadReceipt, |h, c, p| { h.reload(&c, p) };
    ListModels, "model.list", Empty => Vec<ModelDescription>, |h, c, _p| { h.model_list(&c) };
    ListTools, "tool.list", ToolListRequest => Vec<ToolSpec>, |h, c, p| {
        match p.target {
            None => h.tool_list(&c),
            Some(ToolListTarget::Agent(target)) => h.tool_list_for_agent(&c, &target),
            Some(ToolListTarget::Session(target)) => h.tool_list_for_session(&c, &target),
        }
    };
    ReceiptGet, "host.receipt.get", OperationTarget => Receipt, |h, c, p| { h.receipt_get(&c, &p.operation_id) };
    Shutdown, "host.shutdown", HostShutdownRequest => HostShutdownReceipt, |h, c, p| { h.shutdown(&c, p) };
    CreateAgent, "agent.create", AgentCreate => AgentCreateReceipt, |h, c, p| { h.create_agent_async(&c, p).await };
    GetAgent, "agent.get", AgentTarget => AgentInfo, |h, c, p| { h.agent_info(&c, &p.target) };
    ListAgents, "agent.list", Empty => Vec<AgentInfo>, |h, c, _p| { h.agents(&c) };
    StopAgent, "agent.stop", AgentOperation => AgentStopReceipt, |h, c, p| { h.agent_stop(&c, &p.target, AgentStopRequest { operation_id: p.operation_id }) };
    CreateSession, "session.create", SessionCreate => SessionCreateReceipt, |h, c, p| { h.create_session_async(&c, p).await };
    GetSession, "session.get", SessionTarget => SessionInfo, |h, c, p| { h.session_info(&c, &p.target) };
    ListSessions, "session.list", AgentTarget => Vec<SessionInfo>, |h, c, p| { h.sessions(&c, &p.target) };
    SessionState, "session.state", SessionTarget => SessionSnapshot, |h, c, p| { h.session_state(&c, &p.target) };
    LoadSession, "session.load", SessionOperation => SessionControlReceipt, |h, c, p| { h.session_load(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    UnloadSession, "session.unload", SessionOperation => SessionControlReceipt, |h, c, p| { h.session_unload(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    PauseSession, "session.pause", SessionOperation => SessionControlReceipt, |h, c, p| { h.session_pause(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    ResumeSession, "session.resume", SessionOperation => SessionControlReceipt, |h, c, p| { h.session_resume(&c, &p.target, SessionControlRequest { operation_id: p.operation_id }) };
    CancelSession, "session.cancel", CancelSessionParams => SessionCancelReceipt, |h, c, p| { h.session_cancel(&c, &p.target, SessionCancelRequest { operation_id: p.operation_id, keep_inbox: p.keep_inbox }) };
    CloseSession, "session.close", SessionOperation => SessionCloseReceipt, |h, c, p| { h.session_close(&c, &p.target, SessionCloseRequest { operation_id: p.operation_id }) };
    SendMessageOp, "message.send", SendMessage => MessageAccepted, |h, c, p| { h.send(&c, p) };
    GetMessage, "message.get", MessageTargetParams => MessageRecord, |h, c, p| { h.message(&c, &p.target, &p.message_id) };
    GetTurn, "turn.get", TurnTarget => TurnSnapshot, |h, c, p| { h.turn_snapshot(&c, &p.target, &p.turn_id) };
    ListTurns, "turn.list", SessionTarget => Vec<TurnInfo>, |h, c, p| { h.turns(&c, &p.target) };
    WaitTurn, "turn.wait", TurnTarget => TurnResult, |h, c, p| { h.wait_turn(&c, &p.target, &p.turn_id).await };
    CancelTurn, "turn.cancel", CancelTurnParams => TurnCancelReceipt, |h, c, p| { h.turn_cancel(&c, &p.target, TurnCancelRequest { operation_id: p.operation_id, turn_id: p.turn_id, keep_inbox: p.keep_inbox }) };
    GetTurnResult, "turn.result", TurnTarget => TurnResult, |h, c, p| { h.turn_result(&c, &p.target, &p.turn_id) };
    ReadHistory, "session.history.read", HistoryParams => SessionHistorySnapshot, |h, c, p| { h.history_read(&c, &p.target, &p.request).await };
    ReadModelOutput, "turn.output.read", ModelReadParams => ModelTextSnapshot, |h, c, p| { h.turn_output_read(&c, &p.target, &p.turn_id, p.response_index, &p.request) };
    GetToolcall, "toolcall.get", ToolTargetParams => ToolCallSnapshot, |h, c, p| { h.toolcall_get(&c, &p.target, &p.tool) };
    ReadToolOutput, "toolcall.output.read", ToolReadParams => ToolTextSnapshot, |h, c, p| { h.toolcall_output_read(&c, &p.target, &p.tool, &p.request) };
    AnswerInteraction, "interaction.answer", AnswerParams => InteractionAnswerReceipt, |h, c, p| { h.answer_interaction(&c, &p.target, p.request) };
    Unsubscribe, "events.unsubscribe", UnsubscribeRequest => UnsubscribeResult, |h, c, p| { h.events_unsubscribe(&c, &p) };
}

async fn run<O: Operation>(
    host: Arc<Host>,
    caller: CallerContext,
    params: Value,
) -> Result<Value, RpcError> {
    let request: O = serde_json::from_value(params)
        .map_err(|_| protocol_error(-32602, "Invalid method parameters"))?;
    let result = request
        .execute(host, caller)
        .await
        .map_err(RpcError::business)?;
    serde_json::to_value(result)
        .map_err(|_| protocol_error(-32603, "Unable to encode the business result"))
}

pub(crate) fn protocol_error(code: i64, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}
