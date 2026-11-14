//! Shared native JSON configuration, independent of owners and frontends.
//!
//! Loading resolves declarations into protocol values only. Model credentials,
//! resource preparation, stable identities, version checks and publication
//! belong to the Host and its direct owners.
#![forbid(unsafe_code)]

mod directory;
mod host;
mod paths;
mod scaffold;

pub use directory::{AgentDefinition, LoadedConfiguration, load_config_directory};
pub use host::*;
pub use paths::{ConfigPaths, resolve_config_paths};
pub use scaffold::create_config_directory;

pub(crate) fn invalid(message: impl Into<String>) -> minicore_host_protocol::Fault {
    minicore_host_protocol::Fault::new(
        minicore_host_protocol::FaultKind::HostConfigInvalid,
        message,
    )
}
