//! The standard Runtime `agent` tool, independent of clients and connections.
//!
//! Business inputs cannot supply trusted provenance. Turn-bound owner ports
//! perform actual authorization and ownership checks; delegation scopes retain
//! only active relationships and the payload currently being handed off.

mod action;
mod config;
mod core_adapter;
mod local_binding;
mod ports;
mod registry;
mod schema;
mod scope;
mod tool;

pub use action::*;
pub use config::*;
pub use core_adapter::CoreTurnPort;
pub use local_binding::{BoundLocalTools, LocalToolInvocationSink};
pub use ports::*;
pub use registry::{AgentToolFactory, ToolFactory, ToolRegistry};
pub use schema::agent_tool_spec;
pub use scope::{DispatchContext, ScopeCloseAccepted, ScopeManager};
pub use tool::AgentTool;
