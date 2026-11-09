//! Direct typed clients and newline-delimited JSON-RPC over trusted async IO.
//!
//! The embedding application owns Host configuration and establishes the trusted
//! stdio channel. No listener, fixture model, network authentication, version
//! negotiation or production process is started by this library. See the crate
//! README for the exact supported method and framing scope.
//!
//! ```no_run
//! use minicore_host_core::Host;
//! use minicore_host_protocol::{CallError, CallOptions};
//! use minicore_host_transport::HostClient;
//! use std::sync::Arc;
//!
//! async fn inspect(host: Arc<Host>) -> Result<(), CallError> {
//!     let client = HostClient::in_process(host)?;
//!     let info = client.info(CallOptions::default()).await?;
//!     assert_eq!(info.host_id, client.initial_info().host_id);
//!     Ok(())
//! }
//! ```
mod client;
mod deadline;
mod operations;
mod rpc;

pub use client::{HostClient, HostService, SessionClient};
pub use operations::SUPPORTED_METHODS;
pub use rpc::serve_stdio;
