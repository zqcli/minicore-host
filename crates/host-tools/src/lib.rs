//! The standard Runtime `agent` tool, independent of clients and connections.
//!
//! Business inputs cannot supply trusted provenance. Turn-bound owner ports
//! perform actual authorization and ownership checks; delegation scopes retain
//! only active relationships and the payload currently being handed off.

mod action;
mod config;
mod ports;
mod schema;

pub use action::*;
pub use config::*;
pub use ports::*;
pub use schema::agent_tool_spec;
