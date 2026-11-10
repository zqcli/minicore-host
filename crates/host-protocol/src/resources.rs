//! Resource query selection. Results reuse the real Host configuration and
//! Runtime tool declarations rather than introducing another resource catalog.
use crate::{AgentRef, SessionRef};
use serde::{Deserialize, Serialize};

/// Omit the target to list every tool available on this Host. A target reads
/// that owner's committed selection; it does not select a server-side Session.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ToolListTarget>,
}

/// Existing complete owner references. Agent and Session references have
/// disjoint required fields and reject unknown fields, so mixed targets fail
/// rather than dropping a Host or Session qualifier.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolListTarget {
    Agent(AgentRef),
    Session(SessionRef),
}
