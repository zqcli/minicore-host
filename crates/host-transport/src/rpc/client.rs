//! Pending RPC calls and subscription setup share one cancellation association.
//!
//! The pending map, RAII waiter, and setup completion remain together so a late
//! response can release only its original observation without cancelling work.
use super::{
    framing::{FrameReader, FrameWriter, LineReader, LineWriter, write_frames},
    wire::{WireNotification, encode_request, protocol},
};
use crate::{
    diagnostics::Call,
    operations::{Empty, Operation},
};
use minicore_host_core::{ObservationNotification, StateEvent};
use minicore_host_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, BufReader},
    sync::{mpsc, oneshot, watch},
    task::AbortHandle,
};
use tracing::Instrument;

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
                Method::EventsUnsubscribe.as_str(),
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
    pub(crate) async fn initialize(
        &self,
        diagnostic: &mut Call,
    ) -> Result<ConnectionInfo, CallError> {
        self.initialize_with(Empty {}, diagnostic).await
    }
    pub(crate) async fn authenticate(
        &self,
        access_key: &str,
        diagnostic: &mut Call,
    ) -> Result<ConnectionInfo, CallError> {
        self.initialize_with(serde_json::json!({ "access_key": access_key }), diagnostic)
            .await
    }
    async fn initialize_with<P: Serialize>(
        &self,
        params: P,
        diagnostic: &mut Call,
    ) -> Result<ConnectionInfo, CallError> {
        let result = self.request("initialize", params, None, diagnostic).await;
        if let Ok(info) = &result {
            diagnostic.connected(info);
        }
        // The caller's original deadline is outside this future. Only that
        // boundary can distinguish its timeout from a genuinely dropped wait.
        result
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
    pub(crate) async fn call<O: Operation>(
        &self,
        request: O,
        diagnostic: &mut Call,
    ) -> Result<O::Output, CallError> {
        self.request(O::METHOD.as_str(), request, None, diagnostic)
            .await
    }
    pub(crate) async fn subscribe(
        self: &Arc<Self>,
        request: SubscribeRequest,
        diagnostic: &mut Call,
    ) -> Result<RpcSubscription, CallError> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let result = self
            .request(
                Method::EventsSubscribe.as_str(),
                request,
                Some(sender),
                diagnostic,
            )
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
        diagnostic: &mut Call,
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
        let span = diagnostic.span();
        async {
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
        .instrument(span)
        .await
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
fn connection_error(message: impl Into<String>) -> CallError {
    CallError::Connection {
        message: message.into(),
    }
}

#[cfg(test)]
mod observation_correlation_tests;
