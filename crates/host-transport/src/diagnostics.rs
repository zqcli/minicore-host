//! RPC metadata never includes the request, response, credentials or raw errors.
use minicore_host_protocol::{
    AgentId, HostId, MessageId, OperationId, SessionId, ToolCallId, TurnId,
};
use serde::Deserialize;
use serde_json::Value;

// Only typed identity leaves are deserialized. Unknown payload/configuration
// fields are skipped, not retained as a Value tree or copied into diagnostics.
#[derive(Default, Deserialize)]
#[serde(default)]
struct MessageCorrelation {
    message_id: Option<MessageId>,
    in_reply_to: Option<MessageId>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Target {
    host_id: Option<HostId>,
    agent_id: Option<AgentId>,
    session_id: Option<SessionId>,
    turn_id: Option<TurnId>,
    tool_call_id: Option<ToolCallId>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RequestCorrelation {
    operation_id: Option<OperationId>,
    turn_id: Option<TurnId>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Correlation {
    operation_id: Option<OperationId>,
    message_id: Option<MessageId>,
    in_reply_to: Option<MessageId>,
    initial_message: Option<MessageCorrelation>,
    target: Option<Target>,
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
            message_id: identity(value.get("message_id")),
            in_reply_to: identity(value.get("in_reply_to")),
            initial_message: value
                .get("initial_message")
                .map(|message| MessageCorrelation {
                    message_id: identity(message.get("message_id")),
                    in_reply_to: identity(message.get("in_reply_to")),
                }),
            target: value.get("target").map(Target::from_params),
            session_id: identity(value.get("session_id")),
            turn_id: identity(value.get("turn_id")),
            tool: value.get("tool").map(Target::from_params),
            request: value.get("request").map(|request| RequestCorrelation {
                operation_id: identity(request.get("operation_id")),
                turn_id: identity(request.get("turn_id")),
            }),
        }
    }
}

impl Correlation {
    /// Inspect only existing wire bytes. Encoding a request solely for logging
    /// is forbidden; a failed extraction leaves the business call unchanged.
    fn from_encoded(bytes: &[u8]) -> Option<Self> {
        #[derive(Deserialize)]
        struct Envelope {
            params: Correlation,
        }
        serde_json::from_slice::<Envelope>(bytes)
            .ok()
            .map(|envelope| envelope.params)
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
