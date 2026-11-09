use crate::{Fault, FaultKind};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, str::FromStr};

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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImplementationStage {
    Foundation,
    Persistence,
    Configuration,
    Runtime,
    Workspace,
    Observation,
}

/// A method name is a scope commitment, not a claim that a handler exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MethodDescriptor {
    pub method: Method,
    pub interaction: InteractionKind,
    pub access: AccessRequirement,
    pub planned_stage: ImplementationStage,
}

macro_rules! methods {
    ($($variant:ident, $name:literal, $interaction:ident, $access:ident, $stage:ident;)+) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        pub enum Method { $(#[serde(rename = $name)] $variant,)+ }
        impl Method {
            pub const fn as_str(self) -> &'static str { match self {$(Self::$variant => $name,)+} }
            pub const fn descriptor(self) -> MethodDescriptor {
                match self {$(Self::$variant => MethodDescriptor {
                    method:self,interaction:InteractionKind::$interaction,
                    access:AccessRequirement::$access,planned_stage:ImplementationStage::$stage,
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
    HostInfo, "host.info", Query, Read, Foundation;
    HostConfigGet, "host.config.get", Query, Read, Configuration;
    HostReload, "host.reload", Command, Operate, Configuration;
    HostReceiptGet, "host.receipt.get", Query, Read, Foundation;
    HostShutdown, "host.shutdown", Command, Operate, Foundation;
    AgentCreate, "agent.create", CommandWithDelivery, Operate, Foundation;
    AgentGet, "agent.get", Query, Read, Foundation;
    AgentList, "agent.list", Query, Read, Foundation;
    AgentUpdate, "agent.update", Command, Operate, Configuration;
    AgentStart, "agent.start", Command, Operate, Foundation;
    AgentStop, "agent.stop", Command, Operate, Foundation;
    AgentSetDefaultSession, "agent.set_default_session", Command, Operate, Configuration;
    AgentDelete, "agent.delete", Command, Operate, Persistence;
    SessionCreate, "session.create", CommandWithDelivery, Operate, Foundation;
    SessionGet, "session.get", Query, Read, Foundation;
    SessionList, "session.list", Query, Read, Foundation;
    SessionUpdate, "session.update", Command, Operate, Configuration;
    SessionState, "session.state", Query, Read, Foundation;
    SessionLoad, "session.load", Command, Operate, Persistence;
    SessionUnload, "session.unload", Command, Operate, Persistence;
    SessionPause, "session.pause", Command, Operate, Foundation;
    SessionResume, "session.resume", Command, Operate, Foundation;
    SessionCancel, "session.cancel", Command, Operate, Foundation;
    SessionClose, "session.close", Command, Operate, Foundation;
    SessionDelete, "session.delete", Command, Operate, Persistence;
    MessageSend, "message.send", Delivery, Operate, Foundation;
    MessageGet, "message.get", Query, Read, Foundation;
    InputList, "input.list", Query, Read, Foundation;
    InputRemove, "input.remove", Command, Operate, Foundation;
    TurnGet, "turn.get", Query, Read, Runtime;
    TurnList, "turn.list", Query, Read, Runtime;
    TurnWait, "turn.wait", Wait, Read, Runtime;
    TurnCancel, "turn.cancel", Command, Operate, Runtime;
    TurnResult, "turn.result", Query, Read, Runtime;
    TurnOutputRead, "turn.output.read", Query, Read, Runtime;
    ModelList, "model.list", Query, Read, Configuration;
    ToolList, "tool.list", Query, Read, Configuration;
    SessionHistoryRead, "session.history.read", Query, Read, Persistence;
    SessionContext, "session.context", Query, Read, Runtime;
    SessionCompact, "session.compact", Command, Operate, Runtime;
    SessionCompactCancel, "session.compact.cancel", Command, Operate, Runtime;
    WorkspaceFiles, "workspace.files", Query, Read, Workspace;
    WorkspaceRead, "workspace.read", Query, Read, Workspace;
    WorkspaceSearch, "workspace.search", Query, Read, Workspace;
    WorkspaceStatus, "workspace.status", Query, Read, Workspace;
    ChangesList, "changes.list", Query, Read, Workspace;
    ChangesDiff, "changes.diff", Query, Read, Workspace;
    ToolcallGet, "toolcall.get", Query, Read, Runtime;
    ToolcallOutputRead, "toolcall.output.read", Query, Read, Runtime;
    InteractionAnswer, "interaction.answer", Command, Operate, Runtime;
    EventsSubscribe, "events.subscribe", Subscription, Read, Observation;
    EventsUnsubscribe, "events.unsubscribe", Command, OwnSubscription, Observation;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MethodAvailability {
    Implemented {},
    NotImplemented { planned_stage: ImplementationStage },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodCapability {
    pub method: Method,
    pub availability: MethodAvailability,
}

/// The actual service explicitly lists methods backed by real handlers. Every
/// other canonical method remains visibly staged, never a fake-success stub.
/// This contract crate defaults to no handlers and grants no operating rights.
pub fn staged_catalog(implemented: &BTreeSet<Method>) -> Vec<MethodCapability> {
    PUBLIC_METHODS
        .iter()
        .map(|method| MethodCapability {
            method: *method,
            availability: if implemented.contains(method) {
                MethodAvailability::Implemented {}
            } else {
                MethodAvailability::NotImplemented {
                    planned_stage: method.descriptor().planned_stage,
                }
            },
        })
        .collect()
}
pub fn require_implemented(method: Method, implemented: &BTreeSet<Method>) -> Result<(), Fault> {
    if implemented.contains(&method) {
        Ok(())
    } else {
        Err(Fault::new(
            FaultKind::HostMethodUnavailable,
            format!("{} is not implemented in this build", method.as_str()),
        ))
    }
}
