//! Initialization, concurrent request dispatch, and server-owned observation forwarding.
use super::{
    framing::{FrameReader, FrameWriter, LineReader, LineWriter, write_frames},
    wire::{Request, WireNotification},
};
use crate::{
    diagnostics::{self, Call},
    operations::{Empty, dispatch, protocol_error},
};
use minicore_host_core::{ConnectionHandle, Host};
use minicore_host_protocol::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite, BufReader},
    sync::mpsc,
    task::JoinSet,
};
use tracing::Instrument;

#[derive(Clone, Copy)]
pub(crate) enum Initialization {
    TrustedLocal,
    Network,
}
impl Initialization {
    fn connect(self, host: &Host, params: Value) -> Result<ConnectionHandle, Box<RpcError>> {
        match self {
            Self::TrustedLocal => {
                serde_json::from_value::<Empty>(params)
                    .map_err(|_| protocol_error(-32602, "Invalid initialization parameters"))?;
                host.connect()
                    .map_err(|fault| Box::new(RpcError::business(fault)))
            }
            Self::Network => {
                // Deliberately neither Debug nor Serialize. Never return raw
                // authentication parameters or serde diagnostics in failures.
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Authentication {
                    access_key: String,
                }
                let auth = serde_json::from_value::<Authentication>(params)
                    .map_err(|_| protocol_error(-32602, "Invalid authentication parameters"))?;
                host.connect_network(&auth.access_key)
                    .map_err(|fault| Box::new(RpcError::business(fault)))
            }
        }
    }
}

/// Serve a channel whose trust was established by its launching/embedding
/// application. Each UTF-8 JSON object occupies one LF-terminated line. There is
/// no frame-size quota, batch support, version negotiation or JSON authority.
///
/// `initialize` with `{}` parameters and a request ID must complete first. It
/// returns ConnectionInfo and is transport setup, not a resource method. Only
/// validated initialization calls Host::connect. Notifications never dispatch
/// business work, including message.send. Thereafter requests run concurrently
/// and one writer owns the output. EOF/error/drop invalidates the actual handle;
/// cancelling a dispatch future cannot undo work already owned by the Host.
pub async fn serve_stdio<R, W>(host: Arc<Host>, input: R, output: W) -> io::Result<()>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    serve_frames(
        host,
        LineReader {
            reader: BufReader::new(input),
            frame: Vec::new(),
        },
        LineWriter(output),
        Initialization::TrustedLocal,
    )
    .await
}

pub(crate) async fn serve_frames(
    host: Arc<Host>,
    mut input: impl FrameReader,
    output: impl FrameWriter,
    initialization: Initialization,
) -> io::Result<()> {
    let (outgoing, replies) = mpsc::unbounded_channel();
    let writer = write_frames(output, replies);
    tokio::pin!(writer);
    let mut connection: Option<ConnectionHandle> = None;
    let mut requests = JoinSet::new();
    loop {
        let revoked = async {
            match &connection {
                Some(handle) => handle.closed().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = revoked => return Ok(()),
            result = &mut writer => return result,
            read = input.read() => {
                let Some(bytes) = read? else { return Ok(()); };
                let request: Request = match serde_json::from_slice(&bytes) {
                    Ok(request) => request,
                    Err(error) => {
                        let (id, error) = if error.is_syntax() || error.is_eof() {
                            (None, protocol_error(-32700, "Parse error"))
                        } else {
                            let id = serde_json::from_slice::<Value>(&bytes).ok()
                                .and_then(|value| value.get("id").cloned())
                                .and_then(|value| serde_json::from_value(value).ok());
                            (id, protocol_error(-32600, "Invalid Request"))
                        };
                        queue_error(&outgoing, id, error)?;
                        continue;
                    }
                };
                let _ = request.jsonrpc;
                let Some(id) = request.id else { continue; };
                let context = connection.as_ref()
                    .map(|handle| diagnostics::server_context(handle.initial_info()))
                    .unwrap_or_default();
                let mut diagnostic = Call::server(context, &request.method, &request.params);
                if request.method == "initialize" {
                    if connection.is_some() {
                        let error = protocol_error(-32600, "Connection is already initialized");
                        diagnostic.server_error(&error);
                        queue_error(&outgoing, Some(id), error)?;
                    } else {
                        let span = diagnostic.span();
                        match span.in_scope(|| initialization.connect(&host, request.params)) {
                            Ok(handle) => {
                                diagnostic.connected(handle.initial_info());
                                diagnostic.server_succeeded();
                                queue_response(&outgoing, &RpcResponse::success(id, handle.initial_info()))?;
                                connection = Some(handle);
                            }
                            Err(error) => {
                                diagnostic.server_error(&error);
                                queue_error(&outgoing, Some(id), *error)?;
                            }
                        }
                    }
                    continue;
                }
                let Some(handle) = &connection else {
                    let error = protocol_error(-32002, "Connection is not initialized");
                    diagnostic.server_error(&error);
                    queue_error(&outgoing, Some(id), error)?;
                    continue;
                };
                let caller = handle.caller();
                let host = Arc::clone(&host);
                let outgoing = outgoing.clone();
                let span = diagnostic.span();
                requests.spawn(async move {
                    if request.method == Method::EventsSubscribe.as_str() {
                        let subscription = span.in_scope(|| serde_json::from_value::<SubscribeRequest>(request.params)
                            .map_err(|_| protocol_error(-32602, "Invalid method parameters"))
                            .and_then(|params| host.events_subscribe(&caller, params).map_err(RpcError::business)));
                        diagnostic.server_finished(&subscription);
                        // Setup ended. The observation stream has no request span
                        // or diagnostic guard held for its lifetime.
                        drop(diagnostic);
                        drop(span);
                        match subscription {
                            Ok(mut subscription) => {
                                queue_response(&outgoing, &RpcResponse::success(id, subscription.result()))?;
                                while let Some(notification) = subscription.recv().await.map_err(io::Error::other)? {
                                    match serde_json::to_vec(&WireNotification::from(notification)) {
                                        Ok(bytes) => outgoing.send(bytes).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "JSON-RPC writer closed"))?,
                                        Err(_) => subscription.end_after_delivery_failure(Fault::new(FaultKind::HostObservationResyncRequired, "unable to encode an observation payload")),
                                    }
                                }
                                return Ok(());
                            }
                            Err(error) => return queue_error(&outgoing, Some(id), error),
                        }
                    }
                    let result = dispatch(host, caller, &request.method, request.params, &id).instrument(span).await;
                    diagnostic.server_finished(&result);
                    queue_dispatch_result(&outgoing, id, result)
                });
            }
            Some(joined) = requests.join_next(), if !requests.is_empty() => {
                joined.map_err(|_| io::Error::other("JSON-RPC request dispatcher terminated unexpectedly"))??;
            }
        }
    }
}
fn queue_dispatch_result(
    out: &mpsc::UnboundedSender<Vec<u8>>,
    id: RpcId,
    result: Result<Vec<u8>, RpcError>,
) -> io::Result<()> {
    match result {
        Ok(bytes) => queue_bytes(out, bytes),
        Err(error) => queue_error(out, Some(id), error),
    }
}
fn queue_error(
    out: &mpsc::UnboundedSender<Vec<u8>>,
    id: Option<RpcId>,
    error: RpcError,
) -> io::Result<()> {
    queue_response(
        out,
        &RpcResponse::<Value>::Failure(RpcFailure {
            jsonrpc: JsonRpcVersion::V2,
            id,
            error,
        }),
    )
}
fn queue_response<T: Serialize>(
    out: &mpsc::UnboundedSender<Vec<u8>>,
    response: &T,
) -> io::Result<()> {
    queue_bytes(out, serde_json::to_vec(response).map_err(io::Error::other)?)
}
fn queue_bytes(out: &mpsc::UnboundedSender<Vec<u8>>, bytes: Vec<u8>) -> io::Result<()> {
    out.send(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "JSON-RPC writer closed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::encode_success;
    use serde::ser::SerializeStruct;
    use std::cell::Cell;

    struct FailingResult {
        prefix_encoded: Cell<bool>,
    }
    impl Serialize for FailingResult {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut state = serializer.serialize_struct("FailingResult", 2)?;
            state.serialize_field("body", "private partial result")?;
            self.prefix_encoded.set(true);
            Err(serde::ser::Error::custom("private encoding failure"))
        }
    }

    #[test]
    fn mid_result_encoding_failure_queues_only_the_correlated_protocol_error() {
        let (outgoing, mut frames) = mpsc::unbounded_channel();
        let id = RpcId::String("original-call".into());
        let value = FailingResult {
            prefix_encoded: Cell::new(false),
        };
        let result = encode_success(&id, &value);
        assert!(value.prefix_encoded.get());
        assert!(result.is_err());
        queue_dispatch_result(&outgoing, id.clone(), result).unwrap();

        let bytes = frames.try_recv().unwrap();
        let response: RpcResponse<Value> = serde_json::from_slice(&bytes).unwrap();
        let RpcResponse::Failure(failure) = response else {
            panic!("failed result encoding must not queue a success frame");
        };
        assert_eq!(failure.id, Some(id));
        assert_eq!(failure.error.code, -32603);
        assert_eq!(
            failure.error.message,
            "Unable to encode the business result"
        );
        assert!(failure.error.data.is_none());
        assert!(!String::from_utf8(bytes).unwrap().contains("private"));
        assert!(matches!(
            frames.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        // This call's encoding fault leaves the shared outgoing channel usable.
        let next = RpcId::Integer(2);
        queue_dispatch_result(&outgoing, next.clone(), encode_success(&next, true)).unwrap();
        assert_eq!(
            serde_json::from_slice::<RpcResponse<bool>>(&frames.try_recv().unwrap()).unwrap(),
            RpcResponse::success(next, true)
        );
    }
}
