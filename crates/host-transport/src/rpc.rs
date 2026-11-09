use crate::operations::{Empty, Operation, dispatch, protocol_error};
use minicore_host_core::{
    ConnectionHandle, Host, ObservationEvent, ObservationNotification, StateEvent,
};
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
                    if request.method == "events.subscribe" {
                        let subscription = serde_json::from_value::<SubscribeRequest>(request.params)
                            .map_err(|_| protocol_error(-32602, "Invalid method parameters"))
                            .and_then(|params| host.events_subscribe(&caller, params).map_err(RpcError::business));
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

// Notifications never enter response decoding or consume an RPC request ID.
#[derive(Serialize, Deserialize)]
struct WireNotification {
    jsonrpc: JsonRpcVersion,
    #[serde(flatten)]
    payload: NotificationPayload,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
enum NotificationPayload {
    #[serde(rename = "events.push")]
    Event(Box<SubscriptionPush<ObservationEvent>>),
    #[serde(rename = "events.resync_required")]
    ResyncRequired(ResyncRequired),
}
impl From<ObservationNotification> for WireNotification {
    fn from(value: ObservationNotification) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            payload: match value {
                ObservationNotification::Event(event) => NotificationPayload::Event(event),
                ObservationNotification::ResyncRequired(resync) => {
                    NotificationPayload::ResyncRequired(resync)
                }
            },
        }
    }
}
impl From<WireNotification> for ObservationNotification {
    fn from(value: WireNotification) -> Self {
        match value.payload {
            NotificationPayload::Event(event) => Self::Event(event),
            NotificationPayload::ResyncRequired(resync) => Self::ResyncRequired(resync),
        }
    }
}
fn notification_id(value: &ObservationNotification) -> &SubscriptionId {
    match value {
        ObservationNotification::Event(event) => &event.subscription_id,
        ObservationNotification::ResyncRequired(resync) => &resync.subscription_id,
    }
}

// Only live stream registration and in-flight setup buffering are kept here.
// No business source, replay store, subscription tombstone or global event ID.
type Reply = oneshot::Sender<Result<Value, CallError>>;
type StreamSender = mpsc::UnboundedSender<Result<ObservationNotification, CallError>>;
struct PendingRequest {
    reply: Option<Reply>,
    observation: Option<StreamSender>,
}
struct PendingState {
    next_id: i64,
    calls: BTreeMap<i64, PendingRequest>,
    streams: BTreeMap<SubscriptionId, StreamSender>,
    early: BTreeMap<SubscriptionId, Vec<ObservationNotification>>,
    completed_setups: BTreeMap<i64, SubscriptionId>,
    failure: Option<CallError>,
}
impl PendingState {
    fn allocate_id(&mut self) -> Result<i64, CallError> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| protocol("RPC correlation sequence exhausted"))?;
        Ok(id)
    }
    fn clear_finished_setup(&mut self) {
        if !self.calls.values().any(|call| call.observation.is_some()) {
            self.early.clear();
        }
    }
}
struct Pending {
    state: Mutex<PendingState>,
    closed: watch::Sender<bool>,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
}
impl Pending {
    fn fail(&self, failure: CallError) {
        let mut state = self.state.lock().expect("RPC correlation mutex poisoned");
        if state.failure.is_some() {
            return;
        }
        state.failure = Some(failure.clone());
        for (_, call) in std::mem::take(&mut state.calls) {
            if let Some(sender) = call.reply {
                let _ = sender.send(Err(failure.clone()));
            }
        }
        for (_, sender) in std::mem::take(&mut state.streams) {
            let _ = sender.send(Err(failure.clone()));
        }
        state.early.clear();
        state.completed_setups.clear();
        self.closed.send_replace(true);
    }
    fn unsubscribe(&self, id: &SubscriptionId) {
        let mut state = self.state.lock().expect("RPC correlation mutex poisoned");
        state.streams.remove(id);
        self.queue_unsubscribe(&mut state, id);
    }
    fn queue_unsubscribe(&self, state: &mut PendingState, subscription_id: &SubscriptionId) {
        if state.failure.is_some() {
            return;
        }
        if let Ok(id) = state.allocate_id() {
            // A real request, not a forbidden business notification. Its late
            // response needs no waiter and cannot affect another subscription.
            if let Ok(bytes) = encode_request(
                id,
                "events.unsubscribe",
                UnsubscribeRequest {
                    subscription_id: subscription_id.clone(),
                },
            ) {
                let _ = self.outgoing.send(bytes);
            }
        }
    }
    fn notification(&self, notification: ObservationNotification) {
        let id = notification_id(&notification).clone();
        let ended = matches!(notification, ObservationNotification::ResyncRequired(_));
        let mut state = self.state.lock().expect("RPC correlation mutex poisoned");
        if let Some(sender) = state.streams.get(&id) {
            if sender.send(Ok(notification)).is_err() || ended {
                state.streams.remove(&id);
            }
        } else if state.calls.values().any(|call| call.observation.is_some()) {
            state.early.entry(id).or_default().push(notification);
        }
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
                let mut state = self.state.lock().expect("RPC correlation mutex poisoned");
                if let Some(call) = state.calls.remove(&id) {
                    if let Some(sender) = call.observation {
                        match result.as_ref().ok().and_then(|value| {
                            serde_json::from_value::<SubscribeResult<StateEvent>>(value.clone())
                                .ok()
                        }) {
                            Some(baseline) => {
                                let subscription_id = baseline.subscription_id;
                                if sender.is_closed()
                                    || call.reply.as_ref().is_none_or(oneshot::Sender::is_closed)
                                {
                                    self.queue_unsubscribe(&mut state, &subscription_id);
                                } else {
                                    let mut ended = false;
                                    for notification in
                                        state.early.remove(&subscription_id).unwrap_or_default()
                                    {
                                        ended |= matches!(
                                            notification,
                                            ObservationNotification::ResyncRequired(_)
                                        );
                                        let _ = sender.send(Ok(notification));
                                    }
                                    if !ended {
                                        state.streams.insert(subscription_id.clone(), sender);
                                    }
                                    state.completed_setups.insert(id, subscription_id);
                                }
                            }
                            None => { /* Failed setup has no registered subscription. */ }
                        }
                    }
                    if let Some(sender) = call.reply {
                        let _ = sender.send(result);
                    }
                }
                state.clear_finished_setup();
            }
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
    sent: bool,
}
impl PendingCall {
    fn complete(&self) {
        self.pending
            .state
            .lock()
            .expect("RPC correlation mutex poisoned")
            .completed_setups
            .remove(&self.id);
    }
}
impl Drop for PendingCall {
    fn drop(&mut self) {
        let mut state = self
            .pending
            .state
            .lock()
            .expect("RPC correlation mutex poisoned");
        if let Some(id) = state.completed_setups.remove(&self.id) {
            state.streams.remove(&id);
            self.pending.queue_unsubscribe(&mut state, &id);
        }
        if let Some(call) = state.calls.get_mut(&self.id) {
            if self.sent && call.observation.is_some() {
                // Setup may already have reached the owner. Keep only its
                // correlation until the response can identify what to cancel.
                call.reply = None;
            } else {
                state.calls.remove(&self.id);
            }
        }
        state.clear_finished_setup();
    }
}
pub(crate) struct RpcSubscription {
    result: SubscribeResult<StateEvent>,
    receiver: mpsc::UnboundedReceiver<Result<ObservationNotification, CallError>>,
    client: Arc<RpcClient>,
}
impl RpcSubscription {
    pub(crate) fn result(&self) -> &SubscribeResult<StateEvent> {
        &self.result
    }
    pub(crate) async fn recv(&mut self) -> Result<Option<ObservationNotification>, CallError> {
        self.receiver.recv().await.transpose()
    }
}
impl Drop for RpcSubscription {
    fn drop(&mut self) {
        self.client
            .pending
            .unsubscribe(&self.result.subscription_id);
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
        let (outgoing, lines) = mpsc::unbounded_channel();
        let pending = Arc::new(Pending {
            state: Mutex::new(PendingState {
                next_id: 1,
                calls: BTreeMap::new(),
                streams: BTreeMap::new(),
                early: BTreeMap::new(),
                completed_setups: BTreeMap::new(),
                failure: None,
            }),
            closed,
            outgoing: outgoing.clone(),
        });
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
        self.request("initialize", Empty {}, None).await
    }
    pub(crate) async fn authenticate(&self, access_key: &str) -> Result<ConnectionInfo, CallError> {
        self.request(
            "initialize",
            serde_json::json!({ "access_key": access_key }),
            None,
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
    pub(crate) fn ended_subscription(&self, id: &SubscriptionId) {
        self.pending
            .state
            .lock()
            .expect("RPC correlation mutex poisoned")
            .streams
            .remove(id);
    }
    pub(crate) fn close(&self) {
        self.pending
            .fail(connection_error("JSON-RPC connection closed"));
        self.reader.abort();
        self.writer.abort();
    }
    pub(crate) async fn call<O: Operation>(&self, request: O) -> Result<O::Output, CallError> {
        self.request(O::METHOD, request, None).await
    }
    pub(crate) async fn subscribe(
        self: &Arc<Self>,
        request: SubscribeRequest,
    ) -> Result<RpcSubscription, CallError> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let result = self
            .request("events.subscribe", request, Some(sender))
            .await?;
        Ok(RpcSubscription {
            result,
            receiver,
            client: Arc::clone(self),
        })
    }
    async fn request<P: Serialize, O: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
        observation: Option<StreamSender>,
    ) -> Result<O, CallError> {
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
            let id = state.allocate_id()?;
            state.calls.insert(
                id,
                PendingRequest {
                    reply: Some(sender),
                    observation,
                },
            );
            id
        };
        let mut waiter = PendingCall {
            pending: Arc::clone(&self.pending),
            id,
            sent: false,
        };
        let bytes = encode_request(id, method, params)?;
        self.outgoing
            .send(bytes)
            .map_err(|_| connection_error("JSON-RPC writer closed"))?;
        waiter.sent = true;
        let value = receiver
            .await
            .map_err(|_| connection_error("JSON-RPC response reader closed"))??;
        let result = serde_json::from_value(value)
            .map_err(|error| protocol(&format!("invalid method result: {error}")))?;
        waiter.complete();
        Ok(result)
    }
}
fn encode_request<P: Serialize>(id: i64, method: &str, params: P) -> Result<Vec<u8>, CallError> {
    #[derive(Serialize)]
    struct WireRequest<'a, P> {
        jsonrpc: JsonRpcVersion,
        id: RpcId,
        method: &'a str,
        params: P,
    }
    serde_json::to_vec(&WireRequest {
        jsonrpc: JsonRpcVersion::V2,
        id: RpcId::Integer(id),
        method,
        params,
    })
    .map_err(|error| protocol(&error.to_string()))
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
        let read =
            tokio::select! { _ = closed.changed() => return, result = reader.read() => result };
        match read {
            Ok(None) => {
                pending.fail(connection_error(
                    "JSON-RPC peer disconnected before a complete response",
                ));
                return;
            }
            Err(error) => {
                pending.fail(if error.kind() == io::ErrorKind::InvalidData {
                    protocol(&error.to_string())
                } else {
                    connection_error(error.to_string())
                });
                return;
            }
            Ok(Some(frame)) => {
                let decoded = serde_json::from_slice::<Value>(&frame).ok();
                if let Some(value) = decoded {
                    if value.get("method").is_some() {
                        if let Ok(notification) = serde_json::from_value::<WireNotification>(value)
                        {
                            pending.notification(notification.into());
                            continue;
                        }
                    } else if let Ok(response) = serde_json::from_value::<RpcResponse<Value>>(value)
                    {
                        pending.receive(response);
                        continue;
                    }
                }
                pending.fail(protocol("malformed JSON-RPC response or notification"));
                return;
            }
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

#[cfg(test)]
mod observation_correlation_tests {
    use super::*;
    use minicore_host_core::{BoundResources, ResourceFuture, ResourceResolver};

    struct UnusedResources;
    impl ResourceResolver for UnusedResources {
        fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
            Ok(())
        }
        fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
            Box::pin(async {
                Err::<BoundResources, _>(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "no execution requested in correlation test",
                ))
            })
        }
    }
    fn pending() -> (Arc<Pending>, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (outgoing, frames) = mpsc::unbounded_channel();
        let (closed, _) = watch::channel(false);
        (
            Arc::new(Pending {
                state: Mutex::new(PendingState {
                    next_id: 3,
                    calls: BTreeMap::new(),
                    streams: BTreeMap::new(),
                    early: BTreeMap::new(),
                    completed_setups: BTreeMap::new(),
                    failure: None,
                }),
                outgoing,
                closed,
            }),
            frames,
        )
    }
    type TestReply = oneshot::Receiver<Result<Value, CallError>>;
    type TestEvents = mpsc::UnboundedReceiver<Result<ObservationNotification, CallError>>;
    fn setup(pending: &Pending, id: i64) -> (TestReply, TestEvents) {
        let (reply, result) = oneshot::channel();
        let (observation, events) = mpsc::unbounded_channel();
        pending.state.lock().unwrap().calls.insert(
            id,
            PendingRequest {
                reply: Some(reply),
                observation: Some(observation),
            },
        );
        (result, events)
    }
    #[tokio::test]
    async fn real_host_early_resync_stays_ended_and_old_control_cannot_end_a_new_subscription() {
        let host = Host::memory(Arc::new(UnusedResources)).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let request = SubscribeRequest {
            scope: SubscriptionScope::Host {
                host_id: connection.initial_info().host_id.clone(),
            },
            categories: [EventCategory::State].into_iter().collect(),
        };
        let mut old = host.events_subscribe(&caller, request.clone()).unwrap();
        let (pending, _frames) = pending();
        let (reply, mut events) = setup(&pending, 1);
        // This control frame is produced by the real Host observation owner.
        old.end_after_delivery_failure(Fault::new(
            FaultKind::HostObservationResyncRequired,
            "test adapter confirmed a missed dispatch",
        ));
        let control = old.recv().await.unwrap().unwrap();
        pending.notification(control.clone());
        pending.receive(RpcResponse::success(
            RpcId::Integer(1),
            serde_json::to_value(old.result()).unwrap(),
        ));
        reply.await.unwrap().unwrap();
        assert!(matches!(
            events.recv().await.unwrap().unwrap(),
            ObservationNotification::ResyncRequired(_)
        ));
        assert!(events.recv().await.is_none());
        assert!(pending.state.lock().unwrap().streams.is_empty());
        let fresh = host.events_subscribe(&caller, request).unwrap();
        let (reply, mut events) = setup(&pending, 2);
        pending.receive(RpcResponse::success(
            RpcId::Integer(2),
            serde_json::to_value(fresh.result()).unwrap(),
        ));
        reply.await.unwrap().unwrap();
        pending.notification(control);
        pending.unsubscribe(&old.result().subscription_id);
        assert!(
            pending
                .state
                .lock()
                .unwrap()
                .streams
                .contains_key(&fresh.result().subscription_id)
        );
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn cancelling_setup_after_reader_registers_it_still_releases_only_that_observation() {
        let host = Host::memory(Arc::new(UnusedResources)).unwrap();
        let connection = host.connect().unwrap();
        let subscription = host
            .events_subscribe(
                &connection.caller(),
                SubscribeRequest {
                    scope: SubscriptionScope::Host {
                        host_id: connection.initial_info().host_id.clone(),
                    },
                    categories: [EventCategory::State].into_iter().collect(),
                },
            )
            .unwrap();
        let (pending, mut frames) = pending();
        let (_reply, _events) = setup(&pending, 1);
        let waiter = PendingCall {
            pending: pending.clone(),
            id: 1,
            sent: true,
        };
        pending.receive(RpcResponse::success(
            RpcId::Integer(1),
            serde_json::to_value(subscription.result()).unwrap(),
        ));
        assert!(
            pending
                .state
                .lock()
                .unwrap()
                .streams
                .contains_key(&subscription.result().subscription_id)
        );
        drop(waiter);
        assert!(pending.state.lock().unwrap().streams.is_empty());
        let cleanup: Value = serde_json::from_slice(&frames.recv().await.unwrap()).unwrap();
        assert_eq!(cleanup["method"], "events.unsubscribe");
        assert_eq!(
            cleanup["params"]["subscription_id"],
            subscription.result().subscription_id.to_string()
        );
        assert!(!*pending.closed.borrow());
    }
}
