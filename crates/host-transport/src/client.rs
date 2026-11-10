//! Typed client contexts, with connection and observation lifetimes kept explicit.
mod connection;
mod host;
mod session;
mod subscription;

pub use connection::HostService;
pub use host::HostClient;
pub use session::SessionClient;
pub use subscription::ClientSubscription;
