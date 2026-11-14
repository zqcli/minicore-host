//! Production model/tool/workspace composition, outside the Host owner locks.
//!
//! The builder prepares immutable Host candidates; the resolver binds selected
//! Turn implementations or declaration-only compaction resources; prompt
//! preparation retains frozen Host, Role, Agent and Workspace context.
//! No model requests, business tools, environment scans, dynamic module loading,
//! or process-wide cwd changes occur during preparation.

mod approval;
mod builder;
mod prompt;
mod resolver;
mod workspaces;

pub use builder::{HostCredentials, HostResourceBuilder};

#[cfg(test)]
extern crate self as minicore_host_resources;
