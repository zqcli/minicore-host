//! Direct typed clients, trusted stdio RPC, and authenticated TLS WebSockets.
//!
//! The embedding application owns Host configuration and establishes the trusted
//! stdio channel or supplies a configured WSS listener. No fixture model,
//! version negotiation or production process is started by this library. See the crate
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
mod output;
mod resources;
mod rpc;
mod wss;

pub use client::{ClientSubscription, HostClient, HostService, SessionClient};
pub use operations::SUPPORTED_METHODS;
pub use output::OutputBuffer;
pub use resources::{ToolListRequest, ToolListTarget};
pub use rpc::serve_stdio;
pub use tokio_rustls::rustls as tls;
pub use wss::serve_wss;
