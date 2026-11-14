use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    Query,
    Command,
    CommandWithDelivery,
    Delivery,
    Wait,
    Subscription,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessRequirement {
    Read,
    Operate,
    /// Cancels only this connection's observation; it does not modify execution.
    OwnSubscription,
}
/// A method name is a scope commitment, not a claim that a handler exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MethodDescriptor {
    pub method: Method,
    pub interaction: InteractionKind,
    pub access: AccessRequirement,
}

macro_rules! methods {
    ($($variant:ident, $name:literal, $interaction:ident, $access:ident;)+) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        pub enum Method { $(#[serde(rename = $name)] $variant,)+ }
        impl Method {
            pub const fn as_str(self) -> &'static str { match self {$(Self::$variant => $name,)+} }
            pub const fn descriptor(self) -> MethodDescriptor {
                match self {$(Self::$variant => MethodDescriptor {
                    method:self,interaction:InteractionKind::$interaction,
                    access:AccessRequirement::$access,
                },)+}
            }
        }
        impl FromStr for Method {
            type Err = UnknownMethod;
            fn from_str(value: &str) -> Result<Self,Self::Err> {
                match value {$($name => Ok(Self::$variant),)+ _ => Err(UnknownMethod)}
            }
        }
        pub const PUBLIC_METHODS: &[Method] = &[$(Method::$variant,)+];
        pub const METHOD_CATALOG: &[MethodDescriptor] = &[$(Method::$variant.descriptor(),)+];
    }
}
methods! {
    HostInfo, "host.info", Query, Read;
    HostConfigGet, "host.config.get", Query, Read;
    HostReload, "host.reload", Command, Operate;
    HostReceiptGet, "host.receipt.get", Query, Read;
    HostShutdown, "host.shutdown", Command, Operate;
    AgentCreate, "agent.create", CommandWithDelivery, Operate;
    AgentGet, "agent.get", Query, Read;
    AgentList, "agent.list", Query, Read;
    AgentUpdate, "agent.update", Command, Operate;
    AgentStart, "agent.start", Command, Operate;
    AgentStop, "agent.stop", Command, Operate;
    AgentSetDefaultSession, "agent.set_default_session", Command, Operate;
    AgentDelete, "agent.delete", Command, Operate;
    SessionCreate, "session.create", CommandWithDelivery, Operate;
    SessionGet, "session.get", Query, Read;
    SessionList, "session.list", Query, Read;
    SessionUpdate, "session.update", Command, Operate;
    SessionState, "session.state", Query, Read;
    SessionLoad, "session.load", Command, Operate;
    SessionUnload, "session.unload", Command, Operate;
    SessionPause, "session.pause", Command, Operate;
    SessionResume, "session.resume", Command, Operate;
    SessionCancel, "session.cancel", Command, Operate;
    SessionClose, "session.close", Command, Operate;
    SessionDelete, "session.delete", Command, Operate;
    MessageSend, "message.send", Delivery, Operate;
    MessageGet, "message.get", Query, Read;
    InputList, "input.list", Query, Read;
    InputRemove, "input.remove", Command, Operate;
    TurnGet, "turn.get", Query, Read;
    TurnList, "turn.list", Query, Read;
    TurnWait, "turn.wait", Wait, Read;
    TurnCancel, "turn.cancel", Command, Operate;
    TurnResult, "turn.result", Query, Read;
    TurnOutputRead, "turn.output.read", Query, Read;
    ModelList, "model.list", Query, Read;
    ToolList, "tool.list", Query, Read;
    SessionHistoryRead, "session.history.read", Query, Read;
    SessionContext, "session.context", Query, Read;
    SessionCompact, "session.compact", Command, Operate;
    SessionCompactCancel, "session.compact.cancel", Command, Operate;
    WorkspaceRoots, "workspace.roots", Query, Read;
    WorkspaceList, "workspace.list", Query, Read;
    WorkspaceMkdir, "workspace.mkdir", Command, Operate;
    WorkspaceFiles, "workspace.files", Query, Read;
    WorkspaceRead, "workspace.read", Query, Read;
    WorkspaceSearch, "workspace.search", Query, Read;
    WorkspaceStatus, "workspace.status", Query, Read;
    ChangesList, "changes.list", Query, Read;
    ChangesDiff, "changes.diff", Query, Read;
    ToolcallGet, "toolcall.get", Query, Read;
    ToolcallOutputRead, "toolcall.output.read", Query, Read;
    InteractionAnswer, "interaction.answer", Command, Operate;
    EventsSubscribe, "events.subscribe", Subscription, Read;
    EventsUnsubscribe, "events.unsubscribe", Command, OwnSubscription;
}
impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownMethod;
impl fmt::Display for UnknownMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("method is not in the MiniCore Host public catalog")
    }
}
impl std::error::Error for UnknownMethod {}
