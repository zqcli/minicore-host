use crate::operations::{Empty, Operation, dispatch, protocol_error};
use minicore_host_core::{ConnectionHandle, Host};
use minicore_host_protocol::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot, watch},
    task::{AbortHandle, JoinSet},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    jsonrpc: JsonRpcVersion,
    #[serde(default, deserialize_with = "request_id")]
    id: Option<RpcId>,
    method: String,
    #[serde(default)]
    params: Value,
}
fn request_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<RpcId>, D::Error> {
    RpcId::deserialize(d).map(Some)
}

/// A complete transport message, independent of stdio/WSS framing. No buffer
/// cap is imposed here; each framing adapter enforces only its actual syntax.
pub(crate) trait FrameReader: Send {
    fn read(&mut self) -> impl std::future::Future<Output = io::Result<Option<Vec<u8>>>> + Send;
}
pub(crate) trait FrameWriter: Send {
    fn write(&mut self, bytes: Vec<u8>)
    -> impl std::future::Future<Output = io::Result<()>> + Send;
    fn close(&mut self) -> impl std::future::Future<Output = io::Result<()>> + Send;
}
struct LineReader<R> {
    reader: BufReader<R>,
    frame: Vec<u8>,
}
impl<R: AsyncRead + Unpin + Send> FrameReader for LineReader<R> {
    async fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        // Keep partial bytes in self so cancellation of a select branch cannot
        // drop them or start an independent deadline.
        if self.reader.read_until(b'\n', &mut self.frame).await? == 0 && self.frame.is_empty() {
            return Ok(None);
        }
        if self.frame.last() != Some(&b'\n') {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unterminated JSON-RPC frame",
            ));
        }
        Ok(Some(std::mem::take(&mut self.frame)))
    }
}
struct LineWriter<W>(W);
impl<W: AsyncWrite + Unpin + Send> FrameWriter for LineWriter<W> {
    async fn write(&mut self, mut bytes: Vec<u8>) -> io::Result<()> {
        bytes.push(b'\n');
        self.0.write_all(&bytes).await?;
        self.0.flush().await
    }
    async fn close(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

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
                if request.method == "initialize" {
                    if connection.is_some() {
                        queue_error(&outgoing, Some(id), protocol_error(-32600, "Connection is already initialized"))?;
                    } else {
                        match initialization.connect(&host, request.params) {
                            Ok(handle) => {
                                queue_response(&outgoing, &RpcResponse::success(id, handle.initial_info()))?;
                                connection = Some(handle);
                            }
                            Err(error) => queue_error(&outgoing, Some(id), *error)?,
                        }
                    }
                    continue;
                }
                let Some(handle) = &connection else {
                    queue_error(&outgoing, Some(id), protocol_error(-32002, "Connection is not initialized"))?;
                    continue;
                };
                let caller = handle.caller();
                let host = Arc::clone(&host);
                let outgoing = outgoing.clone();
                requests.spawn(async move {
                    let response = match dispatch(host, caller, &request.method, request.params).await {
                        Ok(result) => RpcResponse::success(id, result),
                        Err(error) => RpcResponse::Failure(RpcFailure { jsonrpc: JsonRpcVersion::V2, id: Some(id), error }),
                    };
                    queue_response(&outgoing, &response)
                });
            }
            Some(joined) = requests.join_next(), if !requests.is_empty() => {
                joined.map_err(|_| io::Error::other("JSON-RPC request dispatcher terminated unexpectedly"))??;
            }
        }
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
    out.send(serde_json::to_vec(response).map_err(io::Error::other)?)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "JSON-RPC writer closed"))
}
async fn write_frames(
    mut writer: impl FrameWriter,
    mut frames: mpsc::UnboundedReceiver<Vec<u8>>,
) -> io::Result<()> {
    while let Some(frame) = frames.recv().await {
        writer.write(frame).await?;
    }
    writer.close().await
}

// This is only per-connection response correlation, not business operation
// retention. A timed-out/dropped waiter removes its sender. Late replies are
// discarded using their original transport ID, never dispatched as requests.
type Reply = oneshot::Sender<Result<Value, CallError>>;
struct PendingState {
    next_id: i64,
    calls: BTreeMap<i64, Reply>,
    failure: Option<CallError>,
}
struct Pending {
    state: Mutex<PendingState>,
    closed: watch::Sender<bool>,
}
impl Pending {
    fn fail(&self, failure: CallError) {
        let mut state = self.state.lock().expect("RPC correlation mutex poisoned");
        if state.failure.is_some() {
            return;
        }
        state.failure = Some(failure.clone());
        for (_, sender) in std::mem::take(&mut state.calls) {
            let _ = sender.send(Err(failure.clone()));
        }
        self.closed.send_replace(true);
    }
    fn receive(&self, response: RpcResponse<Value>) {
        let (id, result) = match response {
            RpcResponse::Success(success) => (Some(success.id), Ok(success.result)),
            RpcResponse::Failure(failure) => {
                let error = match failure.error.data {
                    Some(fault) => CallError::Business(fault),
                    None => CallError::Protocol {
                        message: format!(
                            "JSON-RPC {}: {}",
                            failure.error.code, failure.error.message
                        ),
                    },
                };
                (failure.id, Err(error))
            }
        };
        match id {
            Some(RpcId::Integer(id)) => {
                let sender = self
                    .state
                    .lock()
                    .expect("RPC correlation mutex poisoned")
                    .calls
                    .remove(&id);
                if let Some(sender) = sender {
                    let _ = sender.send(result);
                }
            }
            // String/unknown integer IDs cannot match this client's requests.
            Some(RpcId::String(_)) => {}
            None => self.fail(
                result
                    .err()
                    .unwrap_or_else(|| protocol("uncorrelated RPC response")),
            ),
        }
    }
}
struct PendingCall {
    pending: Arc<Pending>,
    id: i64,
}
impl Drop for PendingCall {
    fn drop(&mut self) {
        self.pending
            .state
            .lock()
            .expect("RPC correlation mutex poisoned")
            .calls
            .remove(&self.id);
    }
}
pub(crate) struct RpcClient {
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    pending: Arc<Pending>,
    reader: AbortHandle,
    writer: AbortHandle,
    initial: Option<ConnectionInfo>,
}
impl RpcClient {
    pub(crate) fn start<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::start_frames(
            LineReader {
                reader: BufReader::new(reader),
                frame: Vec::new(),
            },
            LineWriter(writer),
        )
    }
    pub(crate) fn start_frames(
        reader: impl FrameReader + 'static,
        writer: impl FrameWriter + 'static,
    ) -> Self {
        let (closed, _) = watch::channel(false);
        let pending = Arc::new(Pending {
            state: Mutex::new(PendingState {
                next_id: 1,
                calls: BTreeMap::new(),
                failure: None,
            }),
            closed,
        });
        let (outgoing, lines) = mpsc::unbounded_channel();
        let read_pending = Arc::clone(&pending);
        let reader = tokio::spawn(async move {
            read_responses(reader, read_pending).await;
        })
        .abort_handle();
        let write_pending = Arc::clone(&pending);
        let writer = tokio::spawn(async move {
            let mut closed = write_pending.closed.subscribe();
            if *closed.borrow() { return; }
            tokio::select! {
                _ = closed.changed() => {},
                result = write_frames(writer, lines) => {
                    write_pending.fail(connection_error(result.err().map_or_else(|| "JSON-RPC writer closed".into(), |error| error.to_string())));
                }
            }
        }).abort_handle();
        Self {
            outgoing,
            pending,
            reader,
            writer,
            initial: None,
        }
    }
    pub(crate) async fn initialize(&self) -> Result<ConnectionInfo, CallError> {
        self.request("initialize", Empty {}).await
    }
    pub(crate) async fn authenticate(&self, access_key: &str) -> Result<ConnectionInfo, CallError> {
        self.request(
            "initialize",
            serde_json::json!({ "access_key": access_key }),
        )
        .await
    }
    pub(crate) fn initialized(mut self, initial: ConnectionInfo) -> Self {
        self.initial = Some(initial);
        self
    }
    pub(crate) fn initial_info(&self) -> &ConnectionInfo {
        self.initial.as_ref().expect("initialized client")
    }
    pub(crate) fn close(&self) {
        self.pending
            .fail(connection_error("JSON-RPC connection closed"));
        self.reader.abort();
        self.writer.abort();
    }
    pub(crate) async fn call<O: Operation>(&self, request: O) -> Result<O::Output, CallError> {
        self.request(O::METHOD, request).await
    }
    async fn request<P: Serialize, O: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<O, CallError> {
        #[derive(Serialize)]
        struct WireRequest<'a, P> {
            jsonrpc: JsonRpcVersion,
            id: RpcId,
            method: &'a str,
            params: P,
        }
        let (sender, receiver) = oneshot::channel();
        let id = {
            let mut state = self
                .pending
                .state
                .lock()
                .expect("RPC correlation mutex poisoned");
            if let Some(error) = &state.failure {
                return Err(error.clone());
            }
            let id = state.next_id;
            state.next_id = id
                .checked_add(1)
                .ok_or_else(|| protocol("RPC correlation sequence exhausted"))?;
            state.calls.insert(id, sender);
            id
        };
        let _waiter = PendingCall {
            pending: Arc::clone(&self.pending),
            id,
        };
        let bytes = serde_json::to_vec(&WireRequest {
            jsonrpc: JsonRpcVersion::V2,
            id: RpcId::Integer(id),
            method,
            params,
        })
        .map_err(|error| protocol(&error.to_string()))?;
        self.outgoing
            .send(bytes)
            .map_err(|_| connection_error("JSON-RPC writer closed"))?;
        let value = receiver
            .await
            .map_err(|_| connection_error("JSON-RPC response reader closed"))??;
        serde_json::from_value(value)
            .map_err(|error| protocol(&format!("invalid method result: {error}")))
    }
}
impl Drop for RpcClient {
    fn drop(&mut self) {
        self.close();
    }
}
async fn read_responses(mut reader: impl FrameReader, pending: Arc<Pending>) {
    let mut closed = pending.closed.subscribe();
    if *closed.borrow() {
        return;
    }
    loop {
        let read = tokio::select! {
            _ = closed.changed() => return,
            result = reader.read() => result,
        };
        match read {
            Ok(None) => {
                pending.fail(connection_error(
                    "JSON-RPC peer disconnected before a complete response",
                ));
                return;
            }
            Err(error) => {
                let failure = if error.kind() == io::ErrorKind::InvalidData {
                    protocol(&error.to_string())
                } else {
                    connection_error(error.to_string())
                };
                pending.fail(failure);
                return;
            }
            Ok(Some(frame)) => match serde_json::from_slice::<RpcResponse<Value>>(&frame) {
                Ok(response) => pending.receive(response),
                Err(_) => {
                    pending.fail(protocol("malformed JSON-RPC response"));
                    return;
                }
            },
        }
    }
}
fn protocol(message: &str) -> CallError {
    CallError::Protocol {
        message: message.into(),
    }
}
fn connection_error(message: impl Into<String>) -> CallError {
    CallError::Connection {
        message: message.into(),
    }
}
