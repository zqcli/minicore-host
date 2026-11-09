use crate::{AgentId, HostId, SessionId, ToolCallId, TurnId, ValidationError, invalid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRef {
    pub host_id: HostId,
    pub agent_id: AgentId,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRef {
    pub agent_id: AgentId,
    pub session_id: SessionId,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnRef {
    pub session_id: SessionId,
    pub turn_id: TurnId,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallRef {
    pub turn_id: TurnId,
    pub tool_call_id: ToolCallId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    Persistent,
    Memory,
}
impl StorageMode {
    /// Mode omission inherits the direct Agent, never the sender. No promotion.
    pub fn session_mode(self, requested: Option<Self>) -> Result<Self, ValidationError> {
        let resolved = requested.unwrap_or(self);
        if self == Self::Memory && resolved == Self::Persistent {
            Err(invalid("a memory Agent cannot own a persistent Session"))
        } else {
            Ok(resolved)
        }
    }
}

macro_rules! state {
    ($name:ident { $($variant:ident),+ }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
    };
}
state!(HostLifecycle {
    Starting,
    Running,
    Draining,
    Stopping,
    Stopped
});
state!(AgentLifecycle {
    Creating,
    Active,
    Stopping,
    Stopped,
    Retiring,
    Retired
});
state!(SessionLifecycle {
    Open,
    Closing,
    Closed
});
state!(Scheduling { Auto, Paused });
state!(Residency {
    Unloaded,
    Loading,
    Loaded,
    Unloading
});
state!(TurnState {
    Starting,
    Running,
    Cancelling,
    Finalizing,
    Finished
});
state!(TurnOutcome {
    Completed,
    Failed,
    Cancelled
});
state!(PersistenceState {
    Pending,
    Persisted,
    Failed,
    NotApplicable
});
