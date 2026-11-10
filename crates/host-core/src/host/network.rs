//! Host-owned network authentication and configuration publication boundary.
use super::Host;
use crate::control::{CallerContext, ConnectionHandle};
use minicore_host_protocol::Fault;

impl Host {
    /// Trusted embedding setup, separate from any business JSON interface.
    /// Prefer configured Host construction and `reload` for application setup
    /// so network credentials publish with the complete Host configuration.
    /// Unconfigured network access is disabled. `Some` requires a nonblank secret;
    /// `None` disables it. Actual changes invalidate only network connections.
    /// The process must finish configuration before starting a WSS listener.
    pub fn configure_network_access(&self, access_key: Option<String>) -> Result<(), Fault> {
        self.access.publish_network(None, access_key, || Ok(()))
    }

    /// Trusted compatibility hook for embedding-owned publication.
    /// Prefer `reload`, whose configuration owner validates and publishes the
    /// complete prepared configuration at this same authentication boundary.
    ///
    /// Prepare and fully validate the candidate first. Do not invoke for a
    /// candidate containing a restart-required change. `publish` runs under the
    /// same narrow boundary as authentication and direct-owner acceptance: it
    /// must synchronously verify the configuration version and publish prepared
    /// state, without I/O, child calls, or reentering Host methods. On failure,
    /// the old key and all connections remain unchanged. On success, an actual
    /// key change invalidates old network callers before the boundary releases.
    ///
    /// This checks the original caller's current operation right. It is not a
    /// `host.reload` implementation or a remotely callable key-change interface.
    pub fn publish_network_configuration<T>(
        &self,
        caller: &CallerContext,
        access_key: Option<String>,
        publish: impl FnOnce() -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        self.access
            .publish_network(Some(caller), access_key, publish)
    }

    /// Check trusted configuration before accepting a network listener. A
    /// disabled/missing key is a configuration error, never anonymous access.
    pub fn require_network_access(&self) -> Result<(), Fault> {
        self.access.require_network_enabled()
    }

    /// Authenticate and initialize one actual network connection atomically.
    /// The first valid registration can operate; later ones remain read-only.
    /// Neither a successful handshake nor an earlier key check grants access.
    pub fn connect_network(&self, access_key: &str) -> Result<ConnectionHandle, Fault> {
        self.access.connect_network(access_key)
    }
}
