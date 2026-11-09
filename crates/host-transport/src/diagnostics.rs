//! RPC metadata never includes the request, response, credentials or raw errors.
use minicore_host_diagnostics::{Context, EventData, EventSpec, Scope};
use minicore_host_protocol::{
    AgentId, AgentRef, CallError, ConnectionInfo, Fault, HostId, HostIncarnation, LoadId,
    MessageId, MessageInput, MessageTarget, Method, OperationId, RpcError, SendMessage, SessionId,
    SessionRef, SubscriptionScope, ToolCallId, ToolCallRef, TurnId,
};
use serde_json::Value;
use std::time::Instant;
use tracing::Level;

// Only known identity leaves are read. The server borrows its existing parsed
// parameters; typed clients pass fields directly without any diagnostic parse.
#[derive(Default)]
struct MessageCorrelation {
    message_id: Option<MessageId>,
    in_reply_to: Option<MessageId>,
}
#[derive(Default)]
struct Target {
    host_id: Option<HostId>,
    agent_id: Option<AgentId>,
    session_id: Option<SessionId>,
    turn_id: Option<TurnId>,
    tool_call_id: Option<ToolCallId>,
    load_id: Option<LoadId>,
}
#[derive(Default)]
struct RequestCorrelation {
    operation_id: Option<OperationId>,
    turn_id: Option<TurnId>,
    load_id: Option<LoadId>,
}
#[derive(Default)]
pub(crate) struct Correlation {
    operation_id: Option<OperationId>,
    target_operation_id: Option<OperationId>,
    message_id: Option<MessageId>,
    in_reply_to: Option<MessageId>,
    initial_message: Option<MessageCorrelation>,
    target: Option<Target>,
    scope: Option<Target>,
    session_id: Option<SessionId>,
    turn_id: Option<TurnId>,
    tool: Option<Target>,
    request: Option<RequestCorrelation>,
}

fn identity<T: serde::de::DeserializeOwned>(value: Option<&Value>) -> Option<T> {
    T::deserialize(value?).ok()
}
impl Target {
    fn from_params(value: &Value) -> Self {
        Self {
            host_id: identity(value.get("host_id")),
            agent_id: identity(value.get("agent_id")),
            session_id: identity(value.get("session_id")),
            turn_id: identity(value.get("turn_id")),
            tool_call_id: identity(value.get("tool_call_id")),
            load_id: identity(value.get("load_id")),
        }
    }
}
impl Correlation {
    /// Server validation can reject one leaf while other original identities
    /// remain known. Read each borrowed leaf independently; never echo a bad ID
    /// or allocate another body/configuration tree for diagnostic extraction.
    fn from_params(value: &Value) -> Self {
        Self {
            operation_id: identity(value.get("operation_id")),
            target_operation_id: identity(value.get("target_operation_id")),
            message_id: identity(value.get("message_id")),
            in_reply_to: identity(value.get("in_reply_to")),
            initial_message: value
                .get("initial_message")
                .map(|message| MessageCorrelation {
                    message_id: identity(message.get("message_id")),
                    in_reply_to: identity(message.get("in_reply_to")),
                }),
            target: value.get("target").map(Target::from_params),
            scope: value.get("scope").map(Target::from_params),
            session_id: identity(value.get("session_id")),
            turn_id: identity(value.get("turn_id")),
            tool: value.get("tool").map(Target::from_params),
            request: value.get("request").map(|request| RequestCorrelation {
                operation_id: identity(request.get("operation_id")),
                turn_id: identity(request.get("turn_id")),
                load_id: identity(request.get("load_id")),
            }),
        }
    }
}

// Typed calls pass only their already available identity leaves. They never
// serialize a request or introduce another Operation/dispatch framework.
impl Correlation {
    pub(crate) fn agent(target: &AgentRef) -> Self {
        Self {
            target: Some(Target {
                host_id: Some(target.host_id.clone()),
                agent_id: Some(target.agent_id.clone()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    pub(crate) fn session(target: &SessionRef) -> Self {
        Self {
            target: Some(Target {
                agent_id: Some(target.agent_id.clone()),
                session_id: Some(target.session_id.clone()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    pub(crate) fn operation(mut self, id: &OperationId) -> Self {
        self.operation_id = Some(id.clone());
        self
    }
    pub(crate) fn target_operation(mut self, id: &OperationId) -> Self {
        self.target_operation_id = Some(id.clone());
        self
    }
    pub(crate) fn message(mut self, id: &MessageId, reply: Option<&MessageId>) -> Self {
        self.message_id = Some(id.clone());
        self.in_reply_to = reply.cloned();
        self
    }
    pub(crate) fn initial_message(self, message: Option<&MessageInput>) -> Self {
        match message {
            Some(message) => self.message(&message.message_id, message.in_reply_to.as_ref()),
            None => self,
        }
    }
    pub(crate) fn session_id(mut self, id: Option<&SessionId>) -> Self {
        self.session_id = id.cloned();
        self
    }
    pub(crate) fn turn(mut self, id: &TurnId) -> Self {
        self.turn_id = Some(id.clone());
        self
    }
    pub(crate) fn load(mut self, id: &LoadId) -> Self {
        self.target.get_or_insert_default().load_id = Some(id.clone());
        self
    }
    pub(crate) fn tool(mut self, tool: &ToolCallRef) -> Self {
        self.tool = Some(Target {
            turn_id: Some(tool.turn_id.clone()),
            tool_call_id: Some(tool.tool_call_id.clone()),
            ..Default::default()
        });
        self
    }
    pub(crate) fn send(request: &SendMessage) -> Self {
        let target = match &request.target {
            MessageTarget::AgentDefault { host_id, agent_id } => Target {
                host_id: Some(host_id.clone()),
                agent_id: Some(agent_id.clone()),
                ..Default::default()
            },
            MessageTarget::Session {
                agent_id,
                session_id,
            } => Target {
                agent_id: Some(agent_id.clone()),
                session_id: Some(session_id.clone()),
                ..Default::default()
            },
        };
        Self {
            target: Some(target),
            ..Default::default()
        }
        .message(&request.message_id, request.in_reply_to.as_ref())
    }
    pub(crate) fn subscription(scope: &SubscriptionScope) -> Self {
        let target = match scope {
            SubscriptionScope::Host { host_id } => Target {
                host_id: Some(host_id.clone()),
                ..Default::default()
            },
            SubscriptionScope::Agent { host_id, agent_id } => Target {
                host_id: Some(host_id.clone()),
                agent_id: Some(agent_id.clone()),
                ..Default::default()
            },
            SubscriptionScope::Session {
                agent_id,
                session_id,
            } => Target {
                agent_id: Some(agent_id.clone()),
                session_id: Some(session_id.clone()),
                ..Default::default()
            },
            SubscriptionScope::Turn {
                session_id,
                turn_id,
            } => Target {
                session_id: Some(session_id.clone()),
                turn_id: Some(turn_id.clone()),
                ..Default::default()
            },
        };
        Self {
            target: Some(target),
            ..Default::default()
        }
    }
}

/// One actual invocation's metadata, never a retained owner/request or a
/// business receipt. The direct owner remains responsible for primary ERRORs.
pub(crate) struct Call {
    context: Context,
    peer: Option<(HostId, HostIncarnation)>,
    entities: Target,
    target_operation: Option<OperationId>,
    method: &'static str,
    started: Instant,
    finished: bool,
    client: bool,
}
impl Call {
    pub(crate) fn server(context: Context, method: &str, params: &Value) -> Self {
        let mut call = Self::new(context, None, method, false);
        call.correlate(Correlation::from_params(params));
        call
    }
    pub(crate) fn client(
        info: Option<&ConnectionInfo>,
        method: &str,
        correlation: Correlation,
    ) -> Self {
        let mut call = Self::new(
            Context::default(),
            info.map(|info| (info.host_id.clone(), info.host_incarnation.clone())),
            method,
            true,
        );
        call.correlate(correlation);
        call
    }
    fn new(
        context: Context,
        peer: Option<(HostId, HostIncarnation)>,
        method: &str,
        client: bool,
    ) -> Self {
        Self {
            context,
            peer,
            entities: Target::default(),
            target_operation: None,
            method: if method == "initialize" {
                "initialize"
            } else {
                method.parse::<Method>().map_or("unknown", Method::as_str)
            },
            started: Instant::now(),
            finished: false,
            client,
        }
    }
    fn correlate(&mut self, correlation: Correlation) {
        let initial = correlation.initial_message.unwrap_or_default();
        let request = correlation.request.unwrap_or_default();
        let tool = correlation.tool.unwrap_or_default();
        self.context.operation_id = correlation.operation_id.or(request.operation_id);
        self.context.message_id = correlation.message_id.or(initial.message_id);
        self.context.in_reply_to = correlation.in_reply_to.or(initial.in_reply_to);
        self.target_operation = correlation.target_operation_id;
        self.entities = correlation.target.or(correlation.scope).unwrap_or_default();
        self.entities.session_id = self.entities.session_id.take().or(correlation.session_id);
        self.entities.turn_id = self
            .entities
            .turn_id
            .take()
            .or(correlation.turn_id)
            .or(tool.turn_id)
            .or(request.turn_id);
        self.entities.tool_call_id = self.entities.tool_call_id.take().or(tool.tool_call_id);
        self.entities.load_id = self.entities.load_id.take().or(request.load_id);
    }
    pub(crate) fn connected(&mut self, info: &ConnectionInfo) {
        if self.client {
            self.peer = Some((info.host_id.clone(), info.host_incarnation.clone()));
        } else {
            self.context.host_id = Some(info.host_id.clone());
            self.context.host_incarnation = Some(info.host_incarnation.clone());
        }
    }
    pub(crate) fn span(&self) -> tracing::Span {
        self.context.span()
    }
    fn data(&self) -> EventData {
        let mut data = EventData::default()
            .context(self.context.clone())
            .attribute("method", self.method);
        let target_host = self
            .entities
            .host_id
            .as_ref()
            .or(self.peer.as_ref().map(|peer| &peer.0));
        if let Some(id) = target_host {
            data = data.attribute("target_host_id", id.as_str());
        }
        if let Some((id, incarnation)) = &self.peer
            && target_host == Some(id)
        {
            data = data.attribute("target_host_incarnation", incarnation.as_str());
        }
        if let Some(id) = &self.entities.agent_id {
            data = data.attribute("target_agent_id", id.as_str());
        }
        if let Some(id) = &self.entities.session_id {
            data = data.attribute("target_session_id", id.as_str());
        }
        if let Some(id) = &self.entities.turn_id {
            data = data.attribute("target_turn_id", id.as_str());
        }
        if let Some(id) = &self.entities.tool_call_id {
            data = data.attribute("target_tool_call_id", id.as_str());
        }
        if let Some(id) = &self.entities.load_id {
            data = data.attribute("target_load_id", id.as_str());
        }
        if let Some(id) = &self.target_operation {
            data = data.attribute("target_operation_id", id.as_str());
        }
        data
    }
    fn finish(
        &mut self,
        outcome: &'static str,
        delivery: &'static str,
        fault: Option<&Fault>,
        rpc_code: Option<i64>,
        wait_timeout_ms: Option<u64>,
    ) {
        if self.finished {
            return;
        }
        self.finished = true;
        EventSpec {
            level: Level::DEBUG,
            scope: if self.client {
                Scope::Client
            } else {
                Scope::Host
            },
            component: if self.client {
                "client.rpc"
            } else {
                "host.rpc"
            },
            event: "rpc.call_finished",
            message: "Call boundary ended; the direct owner retains its business outcome",
            attributes: &[
                "method",
                "duration_ms",
                "outcome",
                "delivery",
                "rpc_code",
                "wait_timeout_ms",
                "target_host_id",
                "target_host_incarnation",
                "target_agent_id",
                "target_session_id",
                "target_turn_id",
                "target_tool_call_id",
                "target_load_id",
                "target_operation_id",
            ],
        }
        .emit(|| {
            let mut data = self
                .data()
                .attribute("outcome", outcome)
                .attribute("delivery", delivery);
            if let Ok(duration) = u64::try_from(self.started.elapsed().as_millis()) {
                data = data.attribute("duration_ms", duration);
            }
            if let Some(fault) = fault {
                data = data.fault(fault);
            }
            if let Some(code) = rpc_code {
                data = data.attribute("rpc_code", code);
            }
            if let Some(timeout) = wait_timeout_ms {
                data = data.attribute("wait_timeout_ms", timeout);
            }
            data
        });
    }
    pub(crate) fn client_finished<T>(&mut self, result: &Result<T, CallError>) {
        match result {
            Ok(_) => self.finish("response_received", "confirmed", None, None, None),
            Err(CallError::Business(fault)) => {
                self.finish("business_error", "confirmed", Some(fault), None, None)
            }
            Err(CallError::WaitTimeout { wait_timeout_ms }) => self.finish(
                "wait_timeout",
                "unconfirmed",
                None,
                None,
                Some(*wait_timeout_ms),
            ),
            Err(CallError::Connection { .. }) => {
                self.finish("connection_error", "unconfirmed", None, None, None)
            }
            Err(CallError::Protocol { .. }) => {
                self.finish("protocol_error", "unconfirmed", None, None, None)
            }
        }
    }
    pub(crate) fn server_succeeded(&mut self) {
        self.finish("response_ready", "not_sent", None, None, None);
    }
    pub(crate) fn server_error(&mut self, error: &RpcError) {
        self.finish(
            if error.data.is_some() {
                "business_error"
            } else {
                "protocol_error"
            },
            "not_sent",
            error.data.as_ref(),
            Some(error.code),
            None,
        );
    }
    pub(crate) fn server_finished<T>(&mut self, result: &Result<T, RpcError>) {
        match result {
            Ok(_) => self.server_succeeded(),
            Err(error) => self.server_error(error),
        }
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        // Abandoning a request wait is not cancellation or an owner completion.
        self.finish("wait_abandoned", "unconfirmed", None, None, None);
    }
}

pub(crate) fn server_context(info: &ConnectionInfo) -> Context {
    Context {
        host_id: Some(info.host_id.clone()),
        host_incarnation: Some(info.host_incarnation.clone()),
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
