use minicore_host_protocol::*;
use serde_json::{Value, json};
const ID: &str = "9k3m7x2p5v8c4r6t0w1z";
fn request() -> Value {
    json!({"jsonrpc":"2.0","id":"rpc-1","method":"message.send","params":{"message_id":ID,"mode":"follow_up","text":"Review /repo/app","target":{"kind":"session","agent_id":ID,"session_id":ID}}})
}
fn receipt() -> MessageAccepted {
    MessageAccepted::new(
        ID.parse().unwrap(),
        ID.parse().unwrap(),
        SessionRef {
            agent_id: ID.parse().unwrap(),
            session_id: ID.parse().unwrap(),
        },
        ID.parse().unwrap(),
        MessageAcceptedFacts::new(InputQueue::NextTurn, 12),
    )
}
#[test]
fn rpc_params_is_exact_local_request_and_wait_options_never_travel() {
    let wire = request();
    let rpc: MessageSendRpcRequest = serde_json::from_value(wire.clone()).unwrap();
    let local: SendMessage = serde_json::from_value(wire["params"].clone()).unwrap();
    assert_eq!(rpc.params, local);
    assert_eq!(serde_json::to_value(rpc).unwrap(), wire);
    for key in ["caller", "options", "can_operate", "wait_timeout_ms"] {
        let mut invalid = request();
        invalid[key] = json!({});
        assert!(serde_json::from_value::<MessageSendRpcRequest>(invalid).is_err());
        let mut invalid = request();
        invalid["params"][key] = json!({});
        assert!(serde_json::from_value::<MessageSendRpcRequest>(invalid).is_err());
    }
    let mut wrong = request();
    wrong["method"] = json!("turn.start");
    assert!(serde_json::from_value::<MessageSendRpcRequest>(wrong).is_err());
}
#[test]
fn rpc_success_contains_complete_receipt_and_keeps_start_error_a_fact() {
    let mut accepted = receipt();
    accepted.result.start_error = Some(Fault::new(
        FaultKind::SessionResourceMissing,
        "model was removed",
    ));
    let response = MessageSendRpcResponse::success(RpcId::String("rpc-1".into()), accepted.clone());
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["result"]["kind"], json!("message.send"));
    assert_eq!(wire["result"]["result"]["delivery"], json!("accepted"));
    assert!(wire.get("error").is_none());
    assert!(wire["result"].get("Ok").is_none());
    assert_eq!(
        serde_json::from_value::<MessageSendRpcResponse>(wire).unwrap(),
        response
    );
}
#[test]
fn rpc_protocol_codes_and_fault_codes_are_not_confused() {
    let fault = Fault::new(FaultKind::SessionLifecycleClosed, "Session is closed");
    let response = MessageSendRpcResponse::business_failure(RpcId::Integer(4), fault);
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["error"]["code"], json!(-32000));
    assert_eq!(wire["error"]["data"]["code"], json!(3101));
    assert!(wire.get("result").is_none());
    assert_eq!(
        serde_json::from_value::<MessageSendRpcResponse>(wire).unwrap(),
        response
    );
}
#[test]
fn rpc_result_and_error_are_mutually_exclusive() {
    let mut wire = serde_json::to_value(MessageSendRpcResponse::success(
        RpcId::Integer(1),
        receipt(),
    ))
    .unwrap();
    wire["error"] = json!({"code":-32603,"message":"contradiction"});
    assert!(serde_json::from_value::<MessageSendRpcResponse>(wire).is_err());
    assert!(
        serde_json::from_value::<MessageSendRpcResponse>(json!({"jsonrpc":"2.0","id":1})).is_err()
    );
    let mut wrong = request();
    wrong["jsonrpc"] = json!("1.0");
    assert!(serde_json::from_value::<MessageSendRpcRequest>(wrong).is_err());
}

#[test]
fn protocol_failure_requires_id_but_can_use_null_when_unidentified() {
    let error = json!({"code":-32700,"message":"Parse error"});
    assert!(
        serde_json::from_value::<MessageSendRpcResponse>(json!({"jsonrpc":"2.0","error":error}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<MessageSendRpcResponse>(
            json!({"jsonrpc":"2.0","id":null,"error":error})
        )
        .is_ok()
    );
}
