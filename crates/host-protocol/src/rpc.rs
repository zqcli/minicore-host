//! Thin JSON-RPC wire envelopes. Business types remain identical to local
//! results, and these types contain no caller context or waiting options.
use crate::{Fault, MessageAccepted, MessageSendKind, SendMessage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum JsonRpcVersion {
    #[serde(rename = "2.0")]
    V2,
}
/// Transport-only correlation, independent of MessageId and OperationId.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    String(String),
    Integer(i64),
}

/// The fixed message.send template from SPEC §10. There is no generic arbitrary
/// method forwarder for model tools, nor a caller-supplied authority field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSendRpcRequest {
    pub jsonrpc: JsonRpcVersion,
    pub id: RpcId,
    pub method: MessageSendKind,
    pub params: SendMessage,
}
impl MessageSendRpcRequest {
    pub fn new(id: RpcId, params: SendMessage) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id,
            method: MessageSendKind::MessageSend,
            params,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcSuccess<T> {
    pub jsonrpc: JsonRpcVersion,
    pub id: RpcId,
    /// The entire typed result, not a serialized local Result::Ok wrapper.
    pub result: T,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcFailure {
    pub jsonrpc: JsonRpcVersion,
    /// Null is reserved for protocol errors that could not identify a request.
    #[serde(deserialize_with = "crate::nullable")]
    pub id: Option<RpcId>,
    pub error: RpcError,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    /// JSON-RPC code, separate from data.code's registered business semantics.
    pub code: i64,
    pub message: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::present"
    )]
    pub data: Option<Fault>,
}
impl RpcError {
    /// The implementation-defined JSON-RPC server-error range carries a full
    /// Fault in data; it never replaces the business code with a protocol code.
    pub fn business(fault: Fault) -> Self {
        Self {
            code: -32000,
            message: "Business operation failed".into(),
            data: Some(fault),
        }
    }
}

/// Strict variants reject a response with both result and error. An accepted
/// receipt with start_error or partial creation facts still belongs to Success.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcResponse<T> {
    Success(RpcSuccess<T>),
    Failure(RpcFailure),
}
impl<T> RpcResponse<T> {
    pub fn success(id: RpcId, result: T) -> Self {
        Self::Success(RpcSuccess {
            jsonrpc: JsonRpcVersion::V2,
            id,
            result,
        })
    }
    pub fn business_failure(id: RpcId, fault: Fault) -> Self {
        Self::Failure(RpcFailure {
            jsonrpc: JsonRpcVersion::V2,
            id: Some(id),
            error: RpcError::business(fault),
        })
    }
}
pub type MessageSendRpcResponse = RpcResponse<MessageAccepted>;
