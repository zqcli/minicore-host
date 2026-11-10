//! Real connection lifetime, setup, and the typed in-process dispatch boundary.
use super::HostClient;
use crate::{
    deadline::Deadline,
    diagnostics::{Call, Correlation},
    operations::Operation,
    rpc::RpcClient,
};
use minicore_host_core::{CallerContext, ConnectionHandle, Host};
use minicore_host_protocol::*;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::Instrument;

/// The existing Host business entrance. Its context comes from an actual
/// ConnectionHandle, never from serialized parameters or a claimed source.
#[derive(Clone)]
pub struct HostService {
    host: Arc<Host>,
}
impl HostService {
    pub fn new(host: Arc<Host>) -> Self {
        Self { host }
    }
}
impl MessageService<CallerContext> for HostService {
    async fn send(
        &self,
        caller: CallerContext,
        request: SendMessage,
    ) -> Result<MessageAccepted, Fault> {
        self.host.send(&caller, request)
    }
}

pub(super) struct LocalConnection {
    pub(super) host: Arc<Host>,
    pub(super) handle: ConnectionHandle,
}
#[derive(Clone)]
pub(super) enum Connection {
    Local(Arc<LocalConnection>),
    Rpc(Arc<RpcClient>),
}

impl HostClient {
    /// Trusted embedding entry. Business calls remain typed end to end.
    pub fn in_process(host: Arc<Host>) -> Result<Self, CallError> {
        let handle = host.connect()?;
        Ok(Self {
            connection: Connection::Local(Arc::new(LocalConnection { host, handle })),
        })
    }

    /// Initialize one trusted stdio channel. The caller supplies actual pipe or
    /// stream halves; this function does not spawn a process or authenticate a
    /// network peer. Initialization itself uses the same single wait budget.
    pub async fn stdio<R, W>(reader: R, writer: W, options: CallOptions) -> Result<Self, CallError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let deadline = Deadline::new(options)?;
        let connection = RpcClient::start(reader, writer);
        let mut diagnostic = Call::client(None, "initialize", Correlation::default());
        let span = diagnostic.span();
        let result = deadline
            .wait(connection.initialize(&mut diagnostic))
            .instrument(span)
            .await;
        diagnostic.client_finished(&result);
        let initial = result?;
        Ok(Self {
            connection: Connection::Rpc(Arc::new(connection.initialized(initial))),
        })
    }

    /// Establish TLS, WebSocket framing, and authenticated initialization using
    /// one monotonic wait budget. Certificate/name validation uses the supplied
    /// rustls configuration; credentials are sent only after TLS succeeds.
    /// No retry or reconnect is automatic, and plain `ws://` is rejected.
    pub async fn wss(
        url: &str,
        tls: Arc<crate::tls::ClientConfig>,
        access_key: &str,
        options: CallOptions,
    ) -> Result<Self, CallError> {
        let deadline = Deadline::new(options)?;
        let mut diagnostic = Call::client(None, "initialize", Correlation::default());
        let span = diagnostic.span();
        let result = deadline
            .wait(crate::wss::connect(url, tls, access_key, &mut diagnostic))
            .instrument(span)
            .await;
        diagnostic.client_finished(&result);
        let client = result?;
        Ok(Self {
            connection: Connection::Rpc(Arc::new(client)),
        })
    }

    /// Initialization facts, not a fresh Host state query or renewable identity.
    pub fn initial_info(&self) -> &ConnectionInfo {
        match &self.connection {
            Connection::Local(local) => local.handle.initial_info(),
            Connection::Rpc(rpc) => rpc.initial_info(),
        }
    }
    /// End this actual connection, including its clones and pending observations.
    /// Already accepted business work remains with the Host and direct owners.
    pub fn close(&self) {
        match &self.connection {
            Connection::Local(local) => local.handle.close(),
            Connection::Rpc(rpc) => rpc.close(),
        }
    }
    pub(super) async fn call<O: Operation>(
        &self,
        request: O,
        options: CallOptions,
        correlate: impl FnOnce(&O) -> Correlation,
    ) -> Result<O::Output, CallError> {
        let deadline = Deadline::new(options)?;
        let mut diagnostic = Call::client(
            Some(self.initial_info()),
            O::METHOD.as_str(),
            correlate(&request),
        );
        let span = diagnostic.span();
        let result = deadline
            .wait(async {
                match &self.connection {
                    Connection::Local(local) => request
                        .execute(Arc::clone(&local.host), local.handle.caller())
                        .await
                        .map_err(CallError::Business),
                    Connection::Rpc(rpc) => rpc.call(request, &mut diagnostic).await,
                }
            })
            .instrument(span)
            .await;
        // Every owner/admission scope ended before this forwarding summary.
        diagnostic.client_finished(&result);
        result
    }
}
