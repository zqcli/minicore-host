//! RPC request envelopes and observation notifications, separate from framing.
use minicore_host_core::{ObservationEvent, ObservationNotification};
use minicore_host_protocol::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) jsonrpc: JsonRpcVersion,
    #[serde(default, deserialize_with = "request_id")]
    pub(super) id: Option<RpcId>,
    pub(super) method: String,
    #[serde(default)]
    pub(super) params: Value,
}
fn request_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<RpcId>, D::Error> {
    RpcId::deserialize(d).map(Some)
}

// Notifications never enter response decoding or consume an RPC request ID.
#[derive(Serialize, Deserialize)]
pub(super) struct WireNotification {
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
pub(super) fn encode_request<P: Serialize>(
    id: i64,
    method: &str,
    params: P,
) -> Result<Vec<u8>, CallError> {
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
pub(super) fn protocol(message: &str) -> CallError {
    CallError::Protocol {
        message: message.into(),
    }
}
